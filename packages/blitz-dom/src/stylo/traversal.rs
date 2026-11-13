//! Running Stylo's style traversal over a document.

use blitz_traits::SmolStr;
use blitz_traits::events::{BlitzAnimationEvent, BlitzTransitionEvent, DomEventData};
use blitz_traits::node_id::NodeId;
use cssparser::ToCss as _;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::StyleThreading;
use crate::document::QueuedAnimationEvent;
use crate::node::Node;
use style::animation::{AnimationState, KeyframesIterationState};
use style::global_style_data::STYLE_THREAD_POOL;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::ComputedValues;
use style::selector_parser::RestyleDamage;
use style::{
    Atom,
    context::{
        RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext,
        StyleContext, ThreadLocalStyleContext,
    },
    dom::{TElement, TNode},
    global_style_data::GLOBAL_STYLE_DATA,
    servo_arc::Arc,
    shared_lock::StylesheetGuards,
    stylist::RuleInclusion,
    thread_state::ThreadState,
    traversal::{DomTraversal, recalc_style_at, resolve_style},
    traversal_flags::TraversalFlags,
};
use style_traits::values::ToCss as _;

use super::dom::BlitzNode;

/// Serialises `Parallel` style traversals across documents. Stylo's per-worker
/// caches (`StyleSharingCache`, `StyleBloom`) live in native thread-locals that
/// a traversal borrows mutably for its whole duration, so two traversals in
/// flight on the shared pool at once panic with `already mutably borrowed`
/// (<https://github.com/DioxusLabs/blitz/issues/430>).
static PARALLEL_TRAVERSAL_LOCK: Mutex<()> = Mutex::new(());

impl crate::document::BaseDocument {
    pub fn resolve_stylist(&mut self, now: f64) {
        style::thread_state::enter(ThreadState::LAYOUT);

        let guard = &self.guard;
        let guards = StylesheetGuards {
            author: &guard.read(),
            ua_or_user: &guard.read(),
        };

        if let Some(root) = self.nodes[self.root_node_id].first_element_child() {
            self.stylist
                .flush(&guards)
                .process_style(BlitzNode(root), Some(&self.snapshots));
        }

        const TIME_EPSILON: f64 = 1e-5;
        let now_effective = now + TIME_EPSILON;

        // Mark actively animating nodes as dirty
        let mut sets = self.animations.sets.write();
        for (key, set) in sets.iter_mut() {
            let node_id = NodeId::from_u64(key.node.id() as u64);

            // Drop animations belonging to nodes that are no longer in the
            // document. A removed element is never restyled, so it would never
            // get a chance to cancel its own animations; an infinite animation
            // would then keep `has_active_animations` set forever and force a
            // redraw every frame. Emptying the set here lets the `retain` below
            // discard it so the flag can clear on this same pass.
            let in_document = self
                .nodes
                .get(node_id)
                .is_some_and(|node| node.flags.is_in_document());
            if !in_document {
                if self.get_node(node_id).is_some() {
                    let pseudo = match &key.pseudo_element {
                        Some(p) => SmolStr::new(p.to_css_string()),
                        None => SmolStr::default(),
                    };
                    for animation in set.animations.iter() {
                        if animation.state == AnimationState::Running
                            || animation.state == AnimationState::Pending
                        {
                            let current_iteration = match animation.iteration_state {
                                KeyframesIterationState::Finite(current, _) => current,
                                KeyframesIterationState::Infinite(current) => current,
                            };
                            let elapsed = ((current_iteration * animation.duration)
                                + (now - animation.started_at).max(0.0))
                                as f32;
                            self.pending_animation_events.push(QueuedAnimationEvent {
                                node_id,
                                event: DomEventData::AnimationCancel(BlitzAnimationEvent {
                                    animation_name: animation.name.as_str().to_string(),
                                    elapsed_time: elapsed,
                                    pseudo_element: pseudo.clone(),
                                }),
                            });
                        }
                    }
                    for transition in set.transitions.iter() {
                        if transition.state == AnimationState::Running
                            || transition.state == AnimationState::Pending
                        {
                            let property_name =
                                transition.property_animation.property_id().to_css_string();
                            let elapsed = (now - transition.start_time)
                                .max(0.0)
                                .min(transition.property_animation.duration)
                                as f32;
                            self.pending_animation_events.push(QueuedAnimationEvent {
                                node_id,
                                event: DomEventData::TransitionCancel(BlitzTransitionEvent {
                                    property_name,
                                    elapsed_time: elapsed,
                                    pseudo_element: pseudo.clone(),
                                }),
                            });
                        }
                    }
                }
                set.animations.clear();
                set.transitions.clear();
                continue;
            }

            self.nodes[node_id].set_restyle_hint(RestyleHint::RESTYLE_SELF);

            let pseudo = match &key.pseudo_element {
                Some(p) => SmolStr::new(p.to_css_string()),
                None => SmolStr::default(),
            };

            for animation in set.animations.iter_mut() {
                if animation.state == AnimationState::Pending
                    && animation.started_at <= now_effective
                {
                    animation.state = AnimationState::Running;
                    let active_duration = match animation.iteration_state {
                        KeyframesIterationState::Finite(_, max) => max * animation.duration,
                        KeyframesIterationState::Infinite(_) => {
                            if animation.duration == 0.0 {
                                0.0
                            } else {
                                f64::INFINITY
                            }
                        }
                    };
                    let elapsed = (-animation.delay).max(0.0).min(active_duration) as f32;
                    self.pending_animation_events.push(QueuedAnimationEvent {
                        node_id,
                        event: DomEventData::AnimationStart(BlitzAnimationEvent {
                            animation_name: animation.name.as_str().to_string(),
                            elapsed_time: elapsed,
                            pseudo_element: pseudo.clone(),
                        }),
                    });

                    if animation.duration == 0.0 || animation.has_ended(now_effective) {
                        animation.state = AnimationState::Finished;
                        self.pending_animation_events.push(QueuedAnimationEvent {
                            node_id,
                            event: DomEventData::AnimationEnd(BlitzAnimationEvent {
                                animation_name: animation.name.as_str().to_string(),
                                elapsed_time: active_duration.max(0.0) as f32,
                                pseudo_element: pseudo.clone(),
                            }),
                        });
                        continue;
                    }
                }

                if animation.state == AnimationState::Running {
                    let mut iterations_crossed = 0;
                    let mut last_iteration_elapsed = 0.0f32;
                    while animation.duration > 0.0 && animation.iterate_if_necessary(now_effective)
                    {
                        iterations_crossed += 1;
                        let current_iteration = match animation.iteration_state {
                            KeyframesIterationState::Finite(current, _) => current,
                            KeyframesIterationState::Infinite(current) => current,
                        };
                        last_iteration_elapsed = (current_iteration * animation.duration) as f32;
                    }

                    if animation.has_ended(now_effective) {
                        animation.state = AnimationState::Finished;
                        let active_duration = match animation.iteration_state {
                            KeyframesIterationState::Finite(_, max) => max * animation.duration,
                            KeyframesIterationState::Infinite(_) => 0.0,
                        };
                        self.pending_animation_events.push(QueuedAnimationEvent {
                            node_id,
                            event: DomEventData::AnimationEnd(BlitzAnimationEvent {
                                animation_name: animation.name.as_str().to_string(),
                                elapsed_time: active_duration as f32,
                                pseudo_element: pseudo.clone(),
                            }),
                        });
                    } else if iterations_crossed > 0 {
                        self.pending_animation_events.push(QueuedAnimationEvent {
                            node_id,
                            event: DomEventData::AnimationIteration(BlitzAnimationEvent {
                                animation_name: animation.name.as_str().to_string(),
                                elapsed_time: last_iteration_elapsed,
                                pseudo_element: pseudo.clone(),
                            }),
                        });
                    }
                }
            }

            for transition in set.transitions.iter_mut() {
                if transition.state == AnimationState::Pending
                    && transition.start_time <= now_effective
                {
                    transition.state = AnimationState::Running;
                    let elapsed = if transition.delay < 0.0 {
                        (-transition.delay).min(transition.property_animation.duration) as f32
                    } else {
                        0.0
                    };
                    let property_name = transition.property_animation.property_id().to_css_string();
                    self.pending_animation_events.push(QueuedAnimationEvent {
                        node_id,
                        event: DomEventData::TransitionStart(BlitzTransitionEvent {
                            property_name: property_name.clone(),
                            elapsed_time: elapsed,
                            pseudo_element: pseudo.clone(),
                        }),
                    });

                    if transition.property_animation.duration == 0.0
                        || transition.has_ended(now_effective)
                    {
                        transition.state = AnimationState::Finished;
                        self.pending_animation_events.push(QueuedAnimationEvent {
                            node_id,
                            event: DomEventData::TransitionEnd(BlitzTransitionEvent {
                                property_name,
                                elapsed_time: transition.property_animation.duration as f32,
                                pseudo_element: pseudo.clone(),
                            }),
                        });
                        continue;
                    }
                }

                if transition.state == AnimationState::Running
                    && transition.has_ended(now_effective)
                {
                    transition.state = AnimationState::Finished;
                    let property_name = transition.property_animation.property_id().to_css_string();
                    self.pending_animation_events.push(QueuedAnimationEvent {
                        node_id,
                        event: DomEventData::TransitionEnd(BlitzTransitionEvent {
                            property_name,
                            elapsed_time: transition.property_animation.duration as f32,
                            pseudo_element: pseudo.clone(),
                        }),
                    });
                }
            }
        }
        drop(sets);

        // Build the style context used by the style traversal
        let context = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(),
            stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards,
            visited_styles_enabled: false,
            animations: self.animations.clone(),
            current_time_for_animations: now,
            snapshot_map: &self.snapshots,
            registered_speculative_painters: &RegisteredPaintersImpl,
        };

        let mut nodes_needing_style_image_flush = Vec::new();
        let mut zero_duration_animations = Vec::new();
        if let Some(root) = self.try_root_element() {
            let token = RecalcStyle::pre_traverse(BlitzNode(root), &context);

            if token.should_traverse() {
                // Style the elements, resolving their data
                let mut traverser = RecalcStyle::new(context);
                // `Sequential` bypasses Stylo's global pool. See `StyleThreading`.
                let parallel = matches!(self.style_threading, StyleThreading::Parallel);
                let _traversal_lock = parallel.then(|| {
                    PARALLEL_TRAVERSAL_LOCK
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                });
                let pool_guard = parallel.then(|| STYLE_THREAD_POOL.pool());
                let rayon_pool = pool_guard.as_ref().and_then(|g| g.as_ref());
                style::driver::traverse_dom(&traverser, token, rayon_pool);
                nodes_needing_style_image_flush =
                    std::mem::take(traverser.nodes_needing_style_image_flush.get_mut().unwrap());
                zero_duration_animations =
                    std::mem::take(traverser.zero_duration_animations.get_mut().unwrap());
            }
        }
        self.pending_style_image_nodes
            .extend(nodes_needing_style_image_flush);

        for (node_id, name) in zero_duration_animations {
            if self
                .completed_zero_duration_animations
                .insert((node_id, name.clone()))
            {
                self.pending_animation_events.push(QueuedAnimationEvent {
                    node_id,
                    event: DomEventData::AnimationStart(BlitzAnimationEvent {
                        animation_name: name.clone(),
                        elapsed_time: 0.0,
                        pseudo_element: SmolStr::default(),
                    }),
                });
                self.pending_animation_events.push(QueuedAnimationEvent {
                    node_id,
                    event: DomEventData::AnimationEnd(BlitzAnimationEvent {
                        animation_name: name,
                        elapsed_time: 0.0,
                        pseudo_element: SmolStr::default(),
                    }),
                });
            }
        }

        for opaque in self.snapshots.keys() {
            let id = NodeId::from_u64(opaque.id() as u64);
            if let Some(node) = self.nodes.get_mut(id) {
                node.set_has_snapshot(false);
            }
        }
        self.snapshots.clear();

        let mut sets = self.animations.sets.write();
        for (key, set) in sets.iter_mut() {
            let node_id = NodeId::from_u64(key.node.id() as u64);
            let pseudo = match &key.pseudo_element {
                Some(p) => SmolStr::new(p.to_css_string()),
                None => SmolStr::default(),
            };

            for transition in set.transitions.iter_mut() {
                if transition.is_new {
                    let property_name = transition.property_animation.property_id().to_css_string();
                    let elapsed = if transition.delay < 0.0 {
                        (-transition.delay).min(transition.property_animation.duration) as f32
                    } else {
                        0.0
                    };
                    self.pending_animation_events.push(QueuedAnimationEvent {
                        node_id,
                        event: DomEventData::TransitionRun(BlitzTransitionEvent {
                            property_name: property_name.clone(),
                            elapsed_time: elapsed,
                            pseudo_element: pseudo.clone(),
                        }),
                    });
                    transition.is_new = false;

                    // Newly created transition with delay <= 0: promote to Running and check Finished at creation pass t0
                    if transition.state == AnimationState::Pending
                        && transition.start_time <= now_effective
                    {
                        transition.state = AnimationState::Running;
                        self.pending_animation_events.push(QueuedAnimationEvent {
                            node_id,
                            event: DomEventData::TransitionStart(BlitzTransitionEvent {
                                property_name: property_name.clone(),
                                elapsed_time: elapsed,
                                pseudo_element: pseudo.clone(),
                            }),
                        });

                        if transition.property_animation.duration == 0.0
                            || transition.has_ended(now_effective)
                        {
                            transition.state = AnimationState::Finished;
                            self.pending_animation_events.push(QueuedAnimationEvent {
                                node_id,
                                event: DomEventData::TransitionEnd(BlitzTransitionEvent {
                                    property_name,
                                    elapsed_time: transition.property_animation.duration as f32,
                                    pseudo_element: pseudo.clone(),
                                }),
                            });
                        }
                    }
                }

                if transition.state == AnimationState::Canceled {
                    let property_name = transition.property_animation.property_id().to_css_string();
                    let elapsed = (now - transition.start_time)
                        .max(0.0)
                        .min(transition.property_animation.duration)
                        as f32;
                    self.pending_animation_events.push(QueuedAnimationEvent {
                        node_id,
                        event: DomEventData::TransitionCancel(BlitzTransitionEvent {
                            property_name,
                            elapsed_time: elapsed,
                            pseudo_element: pseudo.clone(),
                        }),
                    });
                }
            }

            for animation in set.animations.iter_mut() {
                if animation.is_new {
                    animation.is_new = false;

                    // Newly created animation with delay <= 0: promote to Running and check Finished at creation pass t0
                    if animation.state == AnimationState::Pending
                        && animation.started_at <= now_effective
                    {
                        animation.state = AnimationState::Running;
                        let active_duration = match animation.iteration_state {
                            KeyframesIterationState::Finite(_, max) => max * animation.duration,
                            KeyframesIterationState::Infinite(_) => {
                                if animation.duration == 0.0 {
                                    0.0
                                } else {
                                    f64::INFINITY
                                }
                            }
                        };
                        let elapsed = (-animation.delay).max(0.0).min(active_duration) as f32;
                        self.pending_animation_events.push(QueuedAnimationEvent {
                            node_id,
                            event: DomEventData::AnimationStart(BlitzAnimationEvent {
                                animation_name: animation.name.as_str().to_string(),
                                elapsed_time: elapsed,
                                pseudo_element: pseudo.clone(),
                            }),
                        });

                        if animation.duration == 0.0 || animation.has_ended(now_effective) {
                            animation.state = AnimationState::Finished;
                            self.pending_animation_events.push(QueuedAnimationEvent {
                                node_id,
                                event: DomEventData::AnimationEnd(BlitzAnimationEvent {
                                    animation_name: animation.name.as_str().to_string(),
                                    elapsed_time: active_duration.max(0.0) as f32,
                                    pseudo_element: pseudo.clone(),
                                }),
                            });
                        }
                    }
                }

                if animation.state == AnimationState::Canceled {
                    let current_iteration = match animation.iteration_state {
                        KeyframesIterationState::Finite(current, _) => current,
                        KeyframesIterationState::Infinite(current) => current,
                    };
                    let elapsed = ((current_iteration * animation.duration)
                        + (now - animation.started_at).max(0.0))
                        as f32;
                    self.pending_animation_events.push(QueuedAnimationEvent {
                        node_id,
                        event: DomEventData::AnimationCancel(BlitzAnimationEvent {
                            animation_name: animation.name.as_str().to_string(),
                            elapsed_time: elapsed,
                            pseudo_element: pseudo.clone(),
                        }),
                    });
                }
            }

            set.clear_canceled_animations();
        }
        let mut finished = Vec::new();
        sets.retain(|key, state| {
            let keep = !state.is_empty();
            if !keep {
                finished.push(key.node);
            }
            keep
        });
        // Clear `may_have_animations` on elements whose last set (including
        // their pseudo-elements' sets) was just removed.
        for node in finished {
            if sets.keys().any(|key| key.node == node) {
                continue;
            }
            let node_id = NodeId::from_u64(node.id() as u64);
            if let Some(data) = self.nodes.get(node_id).and_then(|node| node.element_data()) {
                data.may_have_animations.store(false, Ordering::Relaxed);
            }
        }
        self.has_active_animations = sets.values().any(|state| state.needs_animation_ticks());

        // Maybe run garbage collection. Stylo has internal to determine whether to run or not.
        self.stylist.rule_tree().maybe_gc();

        style::thread_state::exit(ThreadState::LAYOUT);
    }

    /// Compute the style of an element which the regular style traversal
    /// skipped because it lives inside a `display: none` subtree.
    ///
    /// The style traversal culls `display: none` subtrees, so such elements
    /// never have styles stored on them, but `getComputedStyle()` must still
    /// return their computed values. This resolves the style on demand
    /// (styling any unstyled ancestors along the way) without storing it on the
    /// node. Returns `None` for non-elements and elements outside the document.
    pub fn resolve_undisplayed_style(&self, node_id: NodeId) -> Option<Arc<ComputedValues>> {
        let node = self.nodes.get(node_id)?;
        if !node.is_element() || !node.flags.is_in_document() {
            return None;
        }

        style::thread_state::enter(ThreadState::LAYOUT);

        let guard = &self.guard;
        let guards = StylesheetGuards {
            author: &guard.read(),
            ua_or_user: &guard.read(),
        };
        let shared = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(),
            stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards,
            visited_styles_enabled: false,
            animations: self.animations.clone(),
            current_time_for_animations: 0.0,
            snapshot_map: &self.snapshots,
            registered_speculative_painters: &RegisteredPaintersImpl,
        };
        // The thread-local context's sequential task list asserts the LAYOUT state when dropped
        let styles = {
            let mut thread_local = ThreadLocalStyleContext::new();
            let mut context = StyleContext {
                shared: &shared,
                thread_local: &mut thread_local,
            };
            resolve_style(
                &mut context,
                BlitzNode(node),
                RuleInclusion::All,
                None,
                None,
            )
        };

        style::thread_state::exit(ThreadState::LAYOUT);

        Some(styles.primary().clone())
    }
}

/// Handle custom painters like images for layouting
///
/// todo: actually implement this
pub struct RegisteredPaintersImpl;
impl RegisteredSpeculativePainters for RegisteredPaintersImpl {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

pub struct RecalcStyle<'a> {
    context: SharedStyleContext<'a>,
    /// Nodes whose `background-image`/`mask-image` layers need flushing to
    /// dedicated storage on the node (see `flush_image_layers_from_style`)
    /// because their style changed during this traversal.
    nodes_needing_style_image_flush: Mutex<Vec<NodeId>>,
    zero_duration_animations: Mutex<Vec<(NodeId, String)>>,
}

impl<'a> RecalcStyle<'a> {
    pub fn new(context: SharedStyleContext<'a>) -> Self {
        RecalcStyle {
            context,
            nodes_needing_style_image_flush: Mutex::new(Vec::new()),
            zero_duration_animations: Mutex::new(Vec::new()),
        }
    }
}

#[allow(unsafe_code)]
impl<'dom> DomTraversal<BlitzNode<'dom>> for RecalcStyle<'_> {
    fn process_preorder<F: FnMut(BlitzNode<'dom>)>(
        &self,
        context: &mut StyleContext<BlitzNode<'dom>>,
        node: BlitzNode<'dom>,
        note_child: F,
    ) {
        if let Some(el) = node.as_element() {
            // let mut data = el.mutate_data().unwrap();
            let mut data = unsafe { el.ensure_data() };
            recalc_style_at(self, context, el, &mut data, note_child);

            sync_pseudo_element_styles(el.0, &data, &self.nodes_needing_style_image_flush);
            if let Some(style) = data.styles.get_primary() {
                el.record_out_of_flow(style);
                let ui = style.get_ui();
                for (i, name) in ui.animation_name_iter().enumerate() {
                    if let Some(atom) = name.as_atom() {
                        let duration = ui.animation_duration_mod(i).seconds();
                        let delay = ui.animation_delay_mod(i).seconds();
                        if duration == 0.0 && delay <= 0.0 {
                            if self.context.stylist.lookup_keyframes(atom, el).is_some() {
                                self.zero_duration_animations
                                    .lock()
                                    .unwrap()
                                    .push((el.id, atom.as_str().to_string()));
                            }
                        }
                    }
                }
            }

            if !data.damage.is_empty() {
                // Mark the ancestor chain so that the damage propagation pass
                // visits this element.
                el.mark_damaged();

                if data
                    .styles
                    .get_primary()
                    .is_some_and(|style| needs_style_image_flush(el.0, style))
                {
                    self.nodes_needing_style_image_flush
                        .lock()
                        .unwrap()
                        .push(el.id);
                }
            }

            // Gets set later on
            el.0.unset_dirty_descendants();
        }
    }

    #[inline]
    fn needs_postorder_traversal() -> bool {
        false
    }

    fn process_postorder(
        &self,
        _style_context: &mut StyleContext<BlitzNode<'dom>>,
        _node: BlitzNode<'dom>,
    ) {
        panic!("this should never be called")
    }

    #[inline]
    fn shared_context(&self) -> &SharedStyleContext<'_> {
        &self.context
    }
}

/// Flush updated pseudo-element (`::before`/`::after`) styles from the owning
/// element's freshly-computed stylo data to the pseudo-element's anonymous node.
///
/// Pseudo-element styles are normally flushed to the pseudo-element's node
/// during box construction (see `flush_pseudo_elements`), but in incremental
/// mode box construction only runs for nodes with construction damage.
/// Pseudo-element style changes which don't require reconstruction (e.g.
/// animations/transitions of repaint- or relayout-only properties) must still
/// be flushed to the pseudo-element's node - along with the damage they imply -
/// so that layout and paint see the new style.
///
/// This runs during the style traversal, immediately after the owning element
/// has been restyled, because that is where the old and new pseudo styles can
/// be diffed precisely (the pseudo node's stored primary style is the "old"
/// style; the owner's just-computed pseudo styles are the "new" ones).
///
/// SAFETY: the style traversal has exclusive access to the tree, and each
/// pseudo-element node is only ever accessed from its owning element's
/// `process_preorder` (pseudo nodes are not part of the DOM child list and are
/// not themselves visited by the traversal), so mutating the pseudo node's
/// stylo data here cannot race with any other traversal thread.
#[allow(unsafe_code)]
fn sync_pseudo_element_styles(
    el: &Node,
    data: &style::data::ElementData,
    nodes_needing_style_image_flush: &Mutex<Vec<NodeId>>,
) {
    let before_node_id = el.before();
    let after_node_id = el.after();
    if before_node_id.is_none() && after_node_id.is_none() {
        return;
    }

    // Note: yes these are kinda backwards (see `flush_pseudo_elements`)
    let pseudos = data.styles.pseudos.as_array();
    let before_style = pseudos[1].clone();
    let after_style = pseudos[0].clone();

    // Creation and removal of pseudo-elements is handled during box construction
    // (Stylo generates construction damage for those cases), so only the case
    // where the pseudo-element both was and remains present is handled here.
    for (pe_node_id, pe_style) in [(before_node_id, before_style), (after_node_id, after_style)] {
        let (Some(pe_node_id), Some(pe_style)) = (pe_node_id, pe_style) else {
            continue;
        };
        let pe_node = el.with(pe_node_id);
        let Some(stylo_data) = pe_node.try_stylo_element_data() else {
            continue;
        };
        let mut pe_data = match unsafe { stylo_data.unsafe_stylo_only_mut() } {
            Some(data) => data,
            None => continue,
        };
        let Some(old_style) = pe_data.styles.primary.clone() else {
            continue;
        };
        if std::ptr::eq(&*old_style, &*pe_style) {
            continue;
        }

        let diff = RestyleDamage::compute_style_difference::<BlitzNode>(&old_style, &pe_style);
        if !diff.damage.is_empty() {
            pe_data.damage.insert(diff.damage);
            pe_node.mark_damaged();

            if needs_style_image_flush(pe_node, &pe_style) {
                nodes_needing_style_image_flush
                    .lock()
                    .unwrap()
                    .push(pe_node_id);
            }
        }
        pe_node.record_out_of_flow(&pe_style);
        pe_data.styles.primary = Some(pe_style);
        pe_data.set_restyled();
    }
}

/// Whether a node's `background-image`/`mask-image` layers need flushing to
/// dedicated storage on the node: the URLs referenced by the new style differ
/// from the image data currently stored on the node.
fn needs_style_image_flush(node: &Node, style: &ComputedValues) -> bool {
    use crate::node::ImageResourceData;
    use style::url::ComputedUrl;
    use style::values::computed::image::Image;

    fn out_of_sync(style_images: &[Image], stored: &[Option<ImageResourceData>]) -> bool {
        if style_images.len() != stored.len() {
            // Differing lengths only matter if either side references an image
            // (a style list of `none` layers and empty storage are in sync).
            return style_images
                .iter()
                .any(|image| matches!(image, Image::Url(_)))
                || stored.iter().any(Option::is_some);
        }
        std::iter::zip(style_images, stored).any(|(style_image, stored)| {
            match (style_image, stored) {
                (Image::Url(ComputedUrl::Valid(url)), Some(data)) => **url != *data.url,
                (Image::Url(ComputedUrl::Valid(_)), None) => true,
                (_, Some(_)) => true,
                (_, None) => false,
            }
        })
    }

    node.data.downcast_element().is_some_and(|el| {
        out_of_sync(
            &style.get_background().background_image.0,
            &el.background_images,
        ) || out_of_sync(&style.get_svg().mask_image.0, &el.mask_images)
    })
}
