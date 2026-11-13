use crate::NodeTree;
use crate::events::{DragMode, handle_dom_event};
use crate::layout::construct::ConstructionTask;
use crate::layout::damage::ALL_DAMAGE;
use crate::mutator::ViewportMut;
use crate::net::{
    Resource, ResourceHandler, ResourceLoadResponse, StylesheetHandler, StylesheetLoader,
};
use crate::node::{ImageData, NodeFlags, RasterImageData, SpecialElementData, Status, TextBrush};
use crate::scrolling::ScrollAnimationState;
use crate::selection::TextSelection;
use crate::stylo::{BlitzNode, UserAgentStylesheet, init_stylo_prefs};
use crate::stylo::{DeviceChanges, make_device, make_media_data};
use crate::stylo_to_cursor_icon::stylo_to_cursor_icon;
use crate::traversal::TreeTraverser;
use crate::url::DocumentUrl;
use crate::util::ImageType;
use crate::{
    DocumentConfig, DocumentMutator, DummyHtmlParserProvider, ElementData, EventDriver,
    HtmlParserProvider, Node, NodeData, NoopEventHandler, StyleThreading, TextNodeData,
};
use bitflags::bitflags;
use blitz_traits::devtools::DevtoolSettings;
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, DomEvent, HitResult, MouseEventButtons, PointerCaptureError,
    UiEvent,
};
use blitz_traits::navigation::{DummyNavigationProvider, NavigationProvider};
use blitz_traits::net::{AbortSignal, DummyNetProvider, NetProvider, Request};
use blitz_traits::node_id::NodeId;
use blitz_traits::shell::{DummyShellProvider, ShellProvider, Viewport};
use cursor_icon::CursorIcon;
use linebender_resource_handle::Blob;
use markup5ever::{LocalName, local_name};
use parley::{FontContext, PlainEditorDriver};
use selectors::matching::QuirksMode;
use smallvec::SmallVec;
use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, Bound, HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLockReadGuard, RwLockWriteGuard};
use std::task::{Context as TaskContext, Waker};
use style::Atom;
use style::animation::DocumentAnimationSet;
use style::attr::{AttrIdentifier, AttrValue};
use style::computed_value_flags::ComputedValueFlags;
use style::data::{ElementData as StyloElementData, ElementStyles};
use style::invalidation::element::restyle_hints::RestyleHint;
use style::media_queries::MediaType;
use style::properties::ComputedValues;
use style::properties::style_structs::Font;
use style::selector_parser::ServoElementSnapshot;
use style::servo_arc::Arc as ServoArc;
use style::values::GenericAtomIdent;
use style::values::computed::UserSelect;
use style::values::computed::ui::CursorKind;
use style::{
    device::Device,
    media_queries::MediaList,
    selector_parser::SnapshotMap,
    shared_lock::{SharedRwLock, StylesheetGuards},
    stylesheets::{AllowImportRules, DocumentStyleSheet, Origin, Stylesheet},
    stylist::Stylist,
};
use style_dom::ElementState;
use stylo_taffy::CopyStyleGetters;
use thin_vec::ThinVec;
use url::Url;
use web_time::Instant;

#[cfg(feature = "parallel-construct")]
use thread_local::ThreadLocal;

pub enum DocGuard<'a> {
    Ref(&'a BaseDocument),
    RefCell(std::cell::Ref<'a, BaseDocument>),
    RwLock(RwLockReadGuard<'a, BaseDocument>),
    Mutex(MutexGuard<'a, BaseDocument>),
}

impl Deref for DocGuard<'_> {
    type Target = BaseDocument;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ref(base_document) => base_document,
            Self::RefCell(refcell_guard) => refcell_guard,
            Self::RwLock(rw_lock_read_guard) => rw_lock_read_guard,
            Self::Mutex(mutex_guard) => mutex_guard,
        }
    }
}

pub enum DocGuardMut<'a> {
    Ref(&'a mut BaseDocument),
    RefCell(std::cell::RefMut<'a, BaseDocument>),
    RwLock(RwLockWriteGuard<'a, BaseDocument>),
    Mutex(MutexGuard<'a, BaseDocument>),
}

impl Deref for DocGuardMut<'_> {
    type Target = BaseDocument;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ref(base_document) => base_document,
            Self::RefCell(refcell_guard) => refcell_guard,
            Self::RwLock(rw_lock_read_guard) => rw_lock_read_guard,
            Self::Mutex(mutex_guard) => mutex_guard,
        }
    }
}

impl DerefMut for DocGuardMut<'_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Ref(base_document) => base_document,
            Self::RefCell(refcell_guard) => &mut *refcell_guard,
            Self::RwLock(rw_lock_read_guard) => &mut *rw_lock_read_guard,
            Self::Mutex(mutex_guard) => &mut *mutex_guard,
        }
    }
}

/// Abstraction over wrappers around [`BaseDocument`] to allow for them all to
/// be driven by [`blitz-shell`](https://docs.rs/blitz-shell)
pub trait Document: Any + 'static {
    fn inner(&self) -> DocGuard<'_>;
    fn inner_mut(&mut self) -> DocGuardMut<'_>;

    /// Update the [`Document`] in response to a [`UiEvent`] (click, keypress, etc)
    fn handle_ui_event(&mut self, event: UiEvent) {
        let mut doc = self.inner_mut();
        let mut driver = EventDriver::new(&mut *doc, NoopEventHandler);
        driver.handle_ui_event(event);
    }

    /// Flush and dispatch any queued animation or transition events
    fn flush_animation_events(&mut self) {
        let mut doc = self.inner_mut();
        let mut driver = EventDriver::new(&mut *doc, NoopEventHandler);
        driver.flush_animation_events();
    }

    /// Poll any pending async operations, and flush changes to the underlying [`BaseDocument`]
    fn poll(&mut self, task_context: Option<TaskContext>) -> bool {
        // Default implementation does nothing
        let _ = task_context;
        false
    }

    /// Get the [`Document`]'s id
    fn id(&self) -> usize {
        self.inner().id
    }

    fn set_pointer_capture(
        &mut self,
        node_id: NodeId,
        pointer_id: i32,
    ) -> Result<(), PointerCaptureError> {
        self.inner_mut().set_pointer_capture(node_id, pointer_id)
    }

    fn release_pointer_capture(
        &mut self,
        node_id: NodeId,
        pointer_id: i32,
    ) -> Result<(), PointerCaptureError> {
        self.inner_mut()
            .release_pointer_capture(node_id, pointer_id)
    }

    fn has_pointer_capture(&self, node_id: NodeId, pointer_id: i32) -> bool {
        self.inner().has_pointer_capture(node_id, pointer_id)
    }
}

pub struct PlainDocument(pub BaseDocument);
impl Document for PlainDocument {
    fn inner(&self) -> DocGuard<'_> {
        DocGuard::Ref(&self.0)
    }
    fn inner_mut(&mut self) -> DocGuardMut<'_> {
        DocGuardMut::Ref(&mut self.0)
    }
}

impl Document for BaseDocument {
    fn inner(&self) -> DocGuard<'_> {
        DocGuard::Ref(self)
    }
    fn inner_mut(&mut self) -> DocGuardMut<'_> {
        DocGuardMut::Ref(self)
    }
}

impl Document for Rc<RefCell<BaseDocument>> {
    fn inner(&self) -> DocGuard<'_> {
        DocGuard::RefCell(self.borrow())
    }

    fn inner_mut(&mut self) -> DocGuardMut<'_> {
        DocGuardMut::RefCell(self.borrow_mut())
    }
}

/// How focus was moved to an element. Determines whether the element
/// matches `:focus-visible` (keyboard and script focus show a focus ring;
/// pointer focus only does so for keyboard-input elements).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusSource {
    Keyboard,
    Pointer,
    Script,
}

pub enum DocumentEvent {
    ResourceLoad(ResourceLoadResponse),
    /// A navigation originating from within an iframe's sub-document
    /// (e.g. a link click), to be applied to the iframe identified by `node_id`.
    NavigateIframe {
        node_id: NodeId,
        url: Url,
    },
}

#[derive(Debug, Clone)]
pub struct QueuedAnimationEvent {
    pub node_id: NodeId,
    pub event: blitz_traits::events::DomEventData,
}

pub type EventCaptureHook = Rc<dyn Fn(&BaseDocument, &DomEvent, &[NodeId])>;

pub struct BaseDocument {
    /// ID of the document
    id: usize,

    // Config
    /// Base url for resolving linked resources (stylesheets, images, fonts, etc)
    pub(crate) url: DocumentUrl,
    // Devtool settings. Currently used to render debug overlays
    pub(crate) devtool_settings: DevtoolSettings,
    // Viewport details such as the dimensions, HiDPI scale, and zoom factor,
    pub(crate) viewport: Viewport,
    /// CSS media type used to evaluate `@media` rules.
    pub(crate) media_type: MediaType,
    /// Changes to the stylist [`Device`] that have been requested since the
    /// last [`resolve`](Self::resolve), to be applied (coalesced into a single
    /// device rebuild) at the start of the next resolve.
    pub(crate) pending_device_changes: DeviceChanges,
    /// Document-wide work for the next [`resolve`](Self::resolve).
    pub(crate) flags: DocumentFlags,
    /// Strategy for Stylo's style traversal during `resolve`.
    pub(crate) style_threading: StyleThreading,
    /// Whether incremental layout is enabled for this document. When disabled,
    /// every node is treated as damaged on every `resolve`, so the same damage
    /// pipeline reconstructs and relays out the whole tree.
    pub(crate) incremental_layout: bool,
    /// How deeply this document is nested within other documents
    /// (0 for a root document). Used to limit `<iframe>` nesting depth.
    pub(crate) subdocument_depth: usize,

    // Events
    pub(crate) tx: Sender<DocumentEvent>,
    // rx will always be Some, except temporarily while processing events
    pub(crate) rx: Option<Receiver<DocumentEvent>>,
    pub(crate) capture_hook: Option<EventCaptureHook>,

    /// A slotmap-backed tree of nodes
    ///
    /// We pin the tree to a guarantee to the nodes it creates that the tree is stable in memory.
    /// There is no way to create the tree - publicly or privately - that would invalidate that invariant.
    pub(crate) nodes: Box<NodeTree>,

    /// The id of the root node (a Document node)
    pub(crate) root_node_id: NodeId,

    // Stylo
    /// The Stylo engine
    pub(crate) stylist: Stylist,
    pub(crate) animations: DocumentAnimationSet,
    /// Stylo shared lock
    pub(crate) guard: SharedRwLock,
    /// Stylo invalidation map. We insert into this map prior to mutating nodes.
    pub(crate) snapshots: SnapshotMap,

    // Parley contexts
    /// A Parley font context
    pub font_ctx: Arc<Mutex<parley::FontContext>>,
    #[cfg(feature = "parallel-construct")]
    /// Thread-and-document-local copies to the font context
    pub(crate) thread_font_contexts: ThreadLocal<RefCell<Box<FontContext>>>,
    /// A Parley layout context
    pub(crate) layout_ctx: parley::LayoutContext<TextBrush>,

    /// The real (non-anonymous) node which is currently hovered (if any).
    /// This is never a layout-generated (anonymous) node, so it remains valid
    /// across box-tree reconstruction.
    pub(crate) hover_node_id: Option<NodeId>,
    /// The precise (may be anonymous) layout node under the pointer (if any).
    /// This can be invalidated by box-tree reconstruction, and is re-resolved against
    /// fresh layout at the end of every `resolve` pass.
    pub(crate) hover_hit_node_id: Option<NodeId>,
    /// Whether the node which is currently hovered is a text node/span
    pub(crate) hover_node_is_text: bool,
    /// The last known pointer position in client coordinates (viewport-relative, unscrolled).
    pub(crate) last_client_pointer_position: Option<taffy::Point<f32>>,
    /// The node which is currently focussed (if any)
    pub(crate) focus_node_id: Option<NodeId>,
    /// The node which is currently active via pointer press (if any)
    pub(crate) active_node_id: Option<NodeId>,
    /// The node which is currently armed/active via Space keydown (if any)
    pub(crate) keyboard_active_node_id: Option<NodeId>,
    /// The node which recieved a mousedown event (if any)
    pub(crate) mousedown_node_id: Option<NodeId>,
    /// The last time a mousedown was made (for double-click detection)
    pub(crate) last_mousedown_time: Option<Instant>,
    /// The position where mousedown occurred (for selection drags and double-click detection)
    pub(crate) mousedown_position: taffy::Point<f32>,
    /// How many clicks have been made in quick succession
    pub(crate) click_count: u16,
    /// Whether we're currently in a text selection drag (moved 2px+ from mousedown)
    pub(crate) drag_mode: DragMode,
    /// Active pointers currently known to the document, keyed by DOM `pointerId`.
    pub(crate) active_pointers: HashMap<i32, ActivePointerRecord>,
    /// Pending pointer capture targets to be promoted on the next pointer event.
    pub(crate) pending_pointer_capture_targets: HashMap<i32, PendingCapture>,
    /// Currently active pointer capture targets, keyed by DOM `pointerId`.
    pub(crate) pointer_capture_targets: HashMap<i32, CaptureTarget>,
    /// Pre-resolved click target for the in-flight `PointerUp` default action.
    pub(crate) pending_click_target: Option<Option<NodeId>>,
    /// The scrollbar thumb currently under the pointer, if any
    pub(crate) hovered_scrollbar: Option<crate::node::ScrollbarRef>,
    /// When each scroll container's overlay scrollbars were last shown
    /// (scrolled, or the pointer left the thumb); drives their fade-out
    pub(crate) scrollbar_activity: HashMap<NodeId, Instant>,
    /// Whether and what kind of scroll animation is currently in progress
    pub(crate) scroll_animation: ScrollAnimationState,

    /// Text selection state (for non-input text)
    pub(crate) text_selection: TextSelection,

    // TODO: collapse animating state into a bitflags
    /// Whether there are active CSS animations/transitions (so we should re-render every frame)
    pub(crate) has_active_animations: bool,
    /// Whether there is a `<canvas>` element in the DOM (so we should re-render every frame)
    pub(crate) has_canvas: bool,
    /// Whether there are subdocuments that are animating (so we should re-render every frame)
    pub(crate) subdoc_is_animating: bool,

    /// Map of id attribute values to node IDs for fast lookups.
    /// May contain multiple nodes for the same id: `get_element_by_id`
    /// returns the first in tree order.
    pub(crate) nodes_to_id: HashMap<String, SmallVec<[NodeId; 1]>>,
    /// Map of `<style>` and `<link>` node IDs to their associated stylesheet
    pub(crate) nodes_to_stylesheet: BTreeMap<NodeId, DocumentStyleSheet>,
    /// Incremented whenever `nodes_to_stylesheet` changes (a stylesheet is
    /// added, replaced or removed), so that CSSOM wrappers can detect that
    /// their cached rule data is stale.
    pub(crate) stylesheet_generation: u64,
    /// Map from form control node ID's to their associated forms node ID's
    pub(crate) controls_to_form: HashMap<NodeId, NodeId>,
    /// Nodes that contain sub documents
    pub(crate) sub_document_nodes: HashSet<NodeId>,
    /// Load state (abort controller and in-flight request id) for each
    /// `<iframe>` element whose sub-document is loaded automatically
    pub(crate) iframe_loads: HashMap<NodeId, crate::iframe::IframeLoad>,
    /// Whether the DOM or focus changed since the accessibility tree was last pushed
    pub(crate) has_changes: bool,
    /// Set of changed nodes for updating the accessibility tree
    pub(crate) deferred_construction_nodes: Vec<ConstructionTask>,

    /// Nodes that contain custom widgets
    #[cfg(feature = "custom-widget")]
    pub(crate) custom_widget_nodes: HashSet<NodeId>,
    /// Rendering resources allocated by custom widgets that should be deallocated during the next render
    #[cfg(feature = "custom-widget")]
    pub(crate) pending_resource_deallocations: Vec<anyrender::ResourceId>,

    /// Cache of loaded images, keyed by URL. Allows reusing images across multiple
    /// elements without re-fetching from the network.
    pub(crate) image_cache: HashMap<String, ImageData>,

    /// Tracks in-flight image requests. When an image is being fetched, additional
    /// requests for the same URL are queued here instead of starting new fetches.
    /// Value is a list of (node_id, image_type) pairs waiting for the image.
    pub(crate) pending_images: HashMap<String, Vec<(NodeId, ImageType)>>,

    /// Nodes whose `background-image`/`mask-image` layers need flushing to
    /// dedicated storage on the node because their style changed (populated by
    /// the style traversal and by pseudo-element box construction).
    pub(crate) pending_style_image_nodes: Vec<NodeId>,

    /// Pending CSS animation and transition events queued during style resolution
    #[allow(dead_code)]
    pub pending_animation_events: Vec<QueuedAnimationEvent>,
    pub(crate) completed_zero_duration_animations: HashSet<(NodeId, String)>,

    // Tracks in-flight "critical" resources (e.g. stylesheets linked from the `<head>`),
    // keyed by request id
    pub(crate) pending_critical_resources: HashSet<usize>,

    // Service providers
    /// Network provider. Can be used to fetch assets.
    pub net_provider: Arc<dyn NetProvider>,
    /// Navigation provider. Can be used to navigate to a new page (bubbles up the event
    /// on e.g. clicking a Link)
    pub navigation_provider: Arc<dyn NavigationProvider>,
    /// Shell provider. Can be used to request a redraw or set the cursor icon
    pub shell_provider: Arc<dyn ShellProvider>,
    /// HTML parser provider. Used to parse HTML for setInnerHTML
    pub html_parser_provider: Arc<dyn HtmlParserProvider>,
    /// Carried on every sub-resource `Request` this document issues; aborting
    /// it cancels all in-flight fetches tied to this document. Set via
    /// [`DocumentConfig::abort_signal`].
    pub(crate) abort_signal: Option<AbortSignal>,
    /// Devtools provider notified of DOM mutations. Set via
    /// [`DocumentConfig::devtools_provider`] or [`Self::set_devtools_provider`].
    #[cfg(feature = "devtools")]
    pub(crate) devtools_provider: Option<Arc<dyn blitz_traits::devtools::DevtoolsProvider>>,
    /// Title shown for this document in devtools (instead of its `<title>`).
    /// Set via [`Self::set_devtools_title`].
    #[cfg(feature = "devtools")]
    pub(crate) devtools_title: Option<String>,
    /// Reusable buffers for document-wide passes.
    pub(crate) scratch: DocumentScratch,
}

#[derive(Clone, Debug)]
pub(crate) struct ActivePointerRecord {
    pub(crate) id: BlitzPointerId,
    pub(crate) is_primary: bool,
    pub(crate) buttons: MouseEventButtons,
    pub(crate) last_event: BlitzPointerEvent,
    pub(crate) element_under_pointer: Option<NodeId>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingCapture {
    pub(crate) target: NodeId,
    pub(crate) explicit: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CaptureTarget {
    Connected { target: NodeId, explicit: bool },
    DisconnectedPendingLostCapture,
}

/// Buffers reused across runs of document-wide passes to avoid reallocating
/// them. Recursive passes use them as stacks: each level pushes above the
/// length it found and truncates back to it before returning.
#[derive(Default)]
pub(crate) struct DocumentScratch {
    /// `(paint order, node)` pairs of the paint lists under construction in
    /// `build_paint_tree_impl`.
    pub(crate) keyed_children: Vec<((i32, i32), NodeId)>,
}

bitflags! {
    /// Document-wide work for the next [`resolve`](BaseDocument::resolve).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub(crate) struct DocumentFlags: u8 {
        /// Recompute the transform and scrollable overflow of every node, not
        /// only of the damaged ones. Both are kept in device pixels, so a scale
        /// change makes them stale throughout the tree.
        const RESOLVE_ALL_TRANSFORMS = 0b0000_0001;
    }
}

impl BaseDocument {
    /// Create a new (empty) [`BaseDocument`] with the specified configuration
    pub fn new(config: DocumentConfig) -> Self {
        init_stylo_prefs();

        static ID_GENERATOR: AtomicUsize = AtomicUsize::new(1);

        let id = ID_GENERATOR.fetch_add(1, Ordering::SeqCst);

        let font_ctx = config
            .font_ctx
            .map(|mut font_ctx| {
                font_ctx.source_cache.make_shared();
                // font_ctx.collection.make_shared();
                font_ctx
            })
            .unwrap_or_else(|| {
                use parley::fontique::{Collection, CollectionOptions, SourceCache};
                let mut font_ctx = FontContext {
                    source_cache: SourceCache::new_shared(),
                    collection: Collection::new(CollectionOptions {
                        shared: false,
                        system_fonts: cfg!(all(
                            feature = "system-fonts",
                            not(target_arch = "wasm32")
                        )),
                    }),
                };
                font_ctx
                    .collection
                    .register_fonts(Blob::new(Arc::new(crate::BULLET_FONT) as _), None);
                font_ctx
            });
        let font_ctx = Arc::new(Mutex::new(font_ctx));

        let viewport = config.viewport.unwrap_or_default();
        let media_type = config.media_type.unwrap_or_else(MediaType::screen);
        let device = make_device(&viewport, media_type.clone(), font_ctx.clone());
        let stylist = Stylist::new(device, QuirksMode::NoQuirks);
        let snapshots = SnapshotMap::new();
        let nodes = Box::new(NodeTree::new());
        let guard = SharedRwLock::new();
        let nodes_to_id = HashMap::new();

        let base_url = config
            .base_url
            .and_then(|url| DocumentUrl::from_str(&url).ok())
            .unwrap_or_default();

        let net_provider = config
            .net_provider
            .unwrap_or_else(|| Arc::new(DummyNetProvider));
        let navigation_provider = config
            .navigation_provider
            .unwrap_or_else(|| Arc::new(DummyNavigationProvider));
        let shell_provider = config
            .shell_provider
            .unwrap_or_else(|| Arc::new(DummyShellProvider));
        let html_parser_provider = config
            .html_parser_provider
            .unwrap_or_else(|| Arc::new(DummyHtmlParserProvider));

        let (tx, rx) = channel();

        let mut doc = Self {
            id,
            tx,
            rx: Some(rx),
            capture_hook: None,

            guard,
            nodes,
            root_node_id: NodeId::default(),
            stylist,
            animations: DocumentAnimationSet::default(),
            snapshots,
            nodes_to_id,
            viewport,
            media_type,
            pending_device_changes: DeviceChanges::empty(),
            flags: DocumentFlags::empty(),
            style_threading: config.style_threading,
            incremental_layout: config.incremental.unwrap_or(true),
            subdocument_depth: config.subdocument_depth,
            devtool_settings: DevtoolSettings::default(),
            url: base_url,
            nodes_to_stylesheet: BTreeMap::new(),
            stylesheet_generation: 0,
            font_ctx,
            #[cfg(feature = "parallel-construct")]
            thread_font_contexts: ThreadLocal::new(),
            layout_ctx: parley::LayoutContext::new(),

            hover_node_id: None,
            hover_hit_node_id: None,
            hover_node_is_text: false,
            last_client_pointer_position: None,
            focus_node_id: None,
            active_node_id: None,
            keyboard_active_node_id: None,
            mousedown_node_id: None,
            has_active_animations: false,
            subdoc_is_animating: false,
            has_canvas: false,
            sub_document_nodes: HashSet::new(),
            iframe_loads: HashMap::new(),

            #[cfg(feature = "custom-widget")]
            custom_widget_nodes: HashSet::new(),
            #[cfg(feature = "custom-widget")]
            pending_resource_deallocations: Vec::new(),

            has_changes: false,
            deferred_construction_nodes: Vec::new(),
            image_cache: HashMap::new(),
            pending_images: HashMap::new(),
            pending_style_image_nodes: Vec::new(),
            pending_animation_events: Vec::new(),
            completed_zero_duration_animations: HashSet::new(),
            pending_critical_resources: HashSet::new(),
            controls_to_form: HashMap::new(),
            net_provider,
            navigation_provider,
            shell_provider,
            html_parser_provider,
            abort_signal: config.abort_signal,
            #[cfg(feature = "devtools")]
            devtools_provider: config.devtools_provider,
            #[cfg(feature = "devtools")]
            devtools_title: None,
            last_mousedown_time: None,
            mousedown_position: taffy::Point::ZERO,
            click_count: 0,
            drag_mode: DragMode::None,
            active_pointers: HashMap::new(),
            pending_pointer_capture_targets: HashMap::new(),
            pointer_capture_targets: HashMap::new(),
            pending_click_target: None,
            hovered_scrollbar: None,
            scrollbar_activity: HashMap::new(),
            scroll_animation: ScrollAnimationState::None,
            text_selection: TextSelection::default(),
            scratch: DocumentScratch::default(),
        };

        // Initialise document with root Document node
        doc.root_node_id = doc.create_node(NodeData::Document(Box::default()));
        doc.root_node_mut().flags.insert(NodeFlags::IS_IN_DOCUMENT);

        for sheet in config.ua_stylesheets {
            doc.add_user_agent_stylesheet(sheet);
        }

        // Stylo data on the root node container is needed to render the node
        let stylo_element_data = StyloElementData {
            styles: ElementStyles {
                primary: Some(
                    ComputedValues::initial_values_with_font_override(Font::initial_values())
                        .to_arc(),
                ),
                ..Default::default()
            },
            ..Default::default()
        };
        let stylo_data = doc.root_node_mut().stylo_element_data_mut();
        *stylo_data.ensure_init_mut() = stylo_element_data;

        doc
    }

    /// Set the Document's networking provider
    pub fn set_net_provider(&mut self, net_provider: Arc<dyn NetProvider>) {
        self.net_provider = net_provider;
    }

    /// Set the Document's navigation provider
    pub fn set_navigation_provider(&mut self, navigation_provider: Arc<dyn NavigationProvider>) {
        self.navigation_provider = navigation_provider;
    }

    /// Set the Document's shell provider
    pub fn set_shell_provider(&mut self, shell_provider: Arc<dyn ShellProvider>) {
        self.shell_provider = shell_provider;
    }

    /// Set (or clear) the Document's devtools provider, which is notified of DOM mutations
    #[cfg(feature = "devtools")]
    pub fn set_devtools_provider(
        &mut self,
        devtools_provider: Option<Arc<dyn blitz_traits::devtools::DevtoolsProvider>>,
    ) {
        self.devtools_provider = devtools_provider;
    }

    /// The title shown for this document in devtools, if overridden with
    /// [`Self::set_devtools_title`]
    #[cfg(feature = "devtools")]
    pub fn devtools_title(&self) -> Option<&str> {
        self.devtools_title.as_deref()
    }

    /// Set (or clear) the title shown for this document in devtools (e.g. as
    /// the CDP target title). Defaults to the document's `<title>`, which an
    /// application's UI document may use for its window title instead.
    #[cfg(feature = "devtools")]
    pub fn set_devtools_title(&mut self, title: Option<String>) {
        self.devtools_title = title;
    }

    /// Set the Document's html parser provider
    pub fn set_html_parser_provider(&mut self, html_parser_provider: Arc<dyn HtmlParserProvider>) {
        self.html_parser_provider = html_parser_provider;
    }

    /// Set base url for resolving linked resources (stylesheets, images, fonts, etc)
    pub fn set_base_url(&mut self, url: &str) {
        self.url = DocumentUrl::from(Url::parse(url).unwrap());
    }

    /// The base url used for resolving linked resources (stylesheets, images, fonts, etc)
    pub fn base_url(&self) -> &Url {
        &self.url
    }

    pub fn set_capture_hook(&mut self, hook: Option<EventCaptureHook>) {
        self.capture_hook = hook;
    }

    pub fn capture_hook(&self) -> Option<&EventCaptureHook> {
        self.capture_hook.as_ref()
    }

    pub fn guard(&self) -> &SharedRwLock {
        &self.guard
    }

    pub fn tree(&self) -> &NodeTree {
        &self.nodes
    }

    pub fn id(&self) -> usize {
        self.id
    }

    /// Wrapper around [`crate::net::stamped_request`]. Use the free function
    /// when `&self` would conflict with a held `&mut` borrow on a field.
    pub(crate) fn build_request(&self, url: url::Url) -> Request {
        crate::net::stamped_request(url, self.abort_signal.as_ref())
    }

    pub fn favicon_url(&self) -> Option<String> {
        self.tree().iter().find_map(|(_, node)| {
            let data = &node.data;
            if !data.is_element_with_tag_name(&local_name!("link")) {
                return None;
            }
            let rel = data.attr(local_name!("rel"))?;
            if !rel
                .split_ascii_whitespace()
                .any(|v| v.eq_ignore_ascii_case("icon"))
            {
                return None;
            }
            data.attr(local_name!("href")).map(|s| s.to_string())
        })
    }

    pub fn get_node(&self, node_id: NodeId) -> Option<&Node> {
        self.nodes.get(node_id)
    }

    pub fn get_node_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(node_id)
    }

    pub fn get_focussed_node_id(&self) -> Option<NodeId> {
        self.focus_node_id
            .or(self.try_root_element().map(|el| el.id))
    }

    /// Returns the currently explicitly focused node, or None if focus is cleared.
    pub fn active_focus_node_id(&self) -> Option<NodeId> {
        self.focus_node_id
    }

    pub fn mutate<'doc>(&'doc mut self) -> DocumentMutator<'doc> {
        DocumentMutator::new(self)
    }

    pub fn handle_dom_event<F: FnMut(DomEvent)>(
        &mut self,
        event: &mut DomEvent,
        dispatch_event: F,
    ) {
        handle_dom_event(self, event, dispatch_event)
    }

    /// Flush and dispatch any pending animation and transition events using lock-free `std::mem::take`.
    pub fn flush_animation_events<F: FnMut(DomEvent)>(&mut self, mut dispatch: F) {
        let events = std::mem::take(&mut self.pending_animation_events);
        for queued in events {
            if self.get_node(queued.node_id).is_none() {
                continue;
            }
            let dom_event = DomEvent::new(queued.node_id, queued.event);
            dispatch(dom_event);
        }
    }

    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    /// Find the label's bound input elements:
    /// the element id referenced by the "for" attribute of a given label element
    /// or the first input element which is nested in the label
    /// Note that although there should only be one bound element,
    /// we return all possibilities instead of just the first
    /// in order to allow the caller to decide which one is correct
    pub fn label_bound_input_element(&self, label_node_id: NodeId) -> Option<&Node> {
        let label_element = self.nodes[label_node_id].element_data()?;
        if let Some(target_element_dom_id) = label_element.attr(local_name!("for")) {
            TreeTraverser::new(self)
                .filter_map(|id| {
                    let node = self.get_node(id)?;
                    let element_data = node.element_data()?;
                    if element_data.name.local != local_name!("input") {
                        return None;
                    }
                    let id = element_data.id.as_ref()?;
                    if *id == *target_element_dom_id {
                        Some(node)
                    } else {
                        None
                    }
                })
                .next()
        } else {
            TreeTraverser::new_with_root(self, label_node_id)
                .filter_map(|child_id| {
                    let node = self.get_node(child_id)?;
                    let element_data = node.element_data()?;
                    if element_data.name.local == local_name!("input") {
                        Some(node)
                    } else {
                        None
                    }
                })
                .next()
        }
    }

    pub fn toggle_checkbox(el: &mut ElementData) -> bool {
        let Some(is_checked) = el.checkbox_input_checked() else {
            return false;
        };
        let checked = !is_checked;
        el.set_checkbox_input_checked(checked);

        checked
    }

    pub fn toggle_radio(&mut self, radio_set_name: String, target_radio_id: NodeId) {
        let radio_ids: Vec<NodeId> = self
            .nodes
            .iter()
            .filter_map(|(i, node)| {
                let el = node.data.downcast_element()?;
                (el.attr(local_name!("name")) == Some(&radio_set_name)
                    && el.checkbox_input_checked().is_some())
                .then_some(i)
            })
            .collect();

        for i in radio_ids {
            let checked = i == target_radio_id;
            self.snapshot_node_and(i, ElementState::CHECKED, |node| {
                if let Some(el) = node.element_data_mut() {
                    el.set_checkbox_input_checked(checked);
                }
                node.mark_ancestors_dirty();
            });
        }
    }

    /// Toggle the `open` attribute of a `<details>` element, expanding or
    /// collapsing it. This is the default action triggered when the element's
    /// first `<summary>` child is activated.
    pub fn toggle_details_open(&mut self, details_id: NodeId) {
        use crate::qual_name;

        let node = &self.nodes[details_id];
        if !node.data.is_element_with_tag_name(&local_name!("details")) {
            return;
        }
        let is_open = node.data.has_attr(local_name!("open"));

        // Note: HTML attributes are in the empty (null) namespace, so the
        // QualName must not use the html namespace here, else it won't match
        // an `open` attribute created by the HTML parser.
        let mut mutator = self.mutate();
        if is_open {
            mutator.clear_attribute(details_id, qual_name!("open"));
        } else {
            mutator.set_attribute(details_id, qual_name!("open"), "");
        }
        drop(mutator);

        self.shell_provider.request_redraw();
    }

    pub fn set_style_property(&mut self, node_id: NodeId, name: &str, value: &str) {
        let node = &mut self.nodes[node_id];
        let did_change = node.element_data_mut().unwrap().set_style_property(
            name,
            value,
            &self.guard,
            self.url.url_extra_data(),
        );
        if did_change {
            node.set_restyle_hint(RestyleHint::RESTYLE_STYLE_ATTRIBUTE);
        }
    }

    pub fn remove_style_property(&mut self, node_id: NodeId, name: &str) {
        let node = &mut self.nodes[node_id];
        let did_change = node.element_data_mut().unwrap().remove_style_property(
            name,
            &self.guard,
            self.url.url_extra_data(),
        );
        if did_change {
            node.set_restyle_hint(RestyleHint::RESTYLE_STYLE_ATTRIBUTE);
        }
    }

    pub fn sub_document_node_ids(&self) -> Vec<NodeId> {
        self.sub_document_nodes.iter().copied().collect()
    }

    pub fn set_sub_document(&mut self, node_id: NodeId, sub_document: Box<dyn Document>) {
        self.nodes[node_id]
            .element_data_mut()
            .unwrap()
            .set_sub_document(sub_document);
        self.sub_document_nodes.insert(node_id);
    }

    pub fn remove_sub_document(&mut self, node_id: NodeId) {
        self.nodes[node_id]
            .element_data_mut()
            .unwrap()
            .remove_sub_document();
        self.sub_document_nodes.remove(&node_id);
        if let Some(load) = self.iframe_loads.remove(&node_id) {
            load.abort_controller.abort();
        }
    }

    /// Poll all sub-documents (see [`Document::poll`]), allowing them to make progress
    /// on any pending async operations (e.g. JavaScript timers). Hosts which poll a
    /// wrapper around a [`BaseDocument`] should call this from their `poll` implementation.
    ///
    /// Returns `true` if any sub-document reported changes.
    pub fn poll_subdocuments(&mut self, waker: Option<&Waker>) -> bool {
        let mut has_changes = false;
        for node_id in self.sub_document_nodes.iter().copied() {
            let Some(sub_doc) = self
                .nodes
                .get_mut(node_id)
                .and_then(|node| node.subdoc_mut())
            else {
                continue;
            };
            let task_context = waker.map(TaskContext::from_waker);
            has_changes |= sub_doc.poll(task_context);
        }
        has_changes
    }

    #[cfg(feature = "custom-widget")]
    pub fn custom_widget_node_ids(&self) -> Vec<NodeId> {
        self.custom_widget_nodes.iter().copied().collect()
    }

    #[cfg(feature = "custom-widget")]
    pub fn take_pending_resource_deallocations(&mut self) -> Vec<anyrender::ResourceId> {
        std::mem::take(&mut self.pending_resource_deallocations)
    }

    #[cfg(feature = "custom-widget")]
    pub fn set_custom_widget(&mut self, node_id: NodeId, widget: Box<dyn crate::Widget>) {
        self.nodes[node_id]
            .element_data_mut()
            .unwrap()
            .set_custom_widget(widget);
        self.custom_widget_nodes.insert(node_id);
    }

    #[cfg(feature = "custom-widget")]
    pub fn remove_custom_widget(&mut self, node_id: NodeId) {
        let resources_to_deallocate = self.nodes[node_id]
            .element_data_mut()
            .unwrap()
            .remove_custom_widget();
        self.pending_resource_deallocations
            .extend_from_slice(&resources_to_deallocate);
        self.custom_widget_nodes.remove(&node_id);
    }

    pub fn root_node(&self) -> &Node {
        &self.nodes[self.root_node_id]
    }

    pub fn root_node_mut(&mut self) -> &mut Node {
        &mut self.nodes[self.root_node_id]
    }

    pub fn try_root_element(&self) -> Option<&Node> {
        self.root_node().first_element_child()
    }

    pub fn root_element(&self) -> &Node {
        self.try_root_element().unwrap()
    }

    pub fn create_node(&mut self, node_data: NodeData) -> NodeId {
        let tree_ptr = self.nodes.as_mut() as *mut NodeTree;
        let guard = self.guard.clone();

        let id = self
            .nodes
            .insert_with_key(|id| Node::new(tree_ptr, id, guard, node_data));

        self.has_changes = true;
        id
    }

    /// Remove a node from the node tree, clearing any interaction state
    /// (hover/active/focus/mousedown/selection/drag/scrollbar) that references
    /// it so that stale NodeIds are never dereferenced after the slot is freed.
    pub(crate) fn remove_node_from_tree(&mut self, node_id: NodeId) -> Option<Node> {
        self.clear_interaction_state_for_removed_node(node_id);
        self.nodes.remove(node_id)
    }

    /// The nearest element ancestor of `node_id` that is still in the
    /// document. Used to retarget hover/active state when the node they
    /// reference is removed. Tolerates already-removed ancestors (subtree
    /// teardown proceeds root-first) by giving up and returning `None`.
    fn nearest_surviving_element_ancestor(&self, node_id: NodeId) -> Option<NodeId> {
        let mut current = self.get_node(node_id)?.parent;
        while let Some(id) = current {
            let node = self.get_node(id)?;
            if node.is_element() && node.flags.is_in_document() {
                return Some(id);
            }
            current = node.parent;
        }
        None
    }

    /// Clear any interaction state (hover/active/focus/mousedown/selection/
    /// drag/scrollbar) that references `node_id`, which is being removed from
    /// the document, running the usual teardown steps. `node_id` must still be
    /// present in the slab.
    ///
    /// This matches browser semantics (WebKit `hoveredElementDidDetach` /
    /// `elementInActiveChainDidDetach`, Blink `HoveredElementDetached` /
    /// `ActiveChainNodeDetached`):
    /// - Hover and active retarget to the nearest surviving element ancestor
    ///   as a *transient bridge*: the HOVER/ACTIVE element-state bits along
    ///   the surviving chain stay lit (no one-frame gap in `:hover`/`:active`
    ///   styling), and the subsequent hover diff can unset exactly the right
    ///   bits. Hover is then re-resolved against the pointer position by
    ///   [`Self::refresh_hover`] at the end of the next resolve pass (the
    ///   analogue of WebKit's "fake mouse move"), which corrects the bridge
    ///   value — including cases where the removed node overflowed its
    ///   ancestor's box, so the ancestor was never truly under the pointer.
    /// - Focus resets to the body (encoded as `None`), running blur
    ///   side-effects (clearing focus element state and disabling IME for
    ///   text inputs).
    pub(crate) fn clear_interaction_state_for_removed_node(&mut self, node_id: NodeId) {
        if !self.nodes.contains_key(node_id) {
            return;
        }

        if self.hover_node_id == Some(node_id) {
            self.hover_node_id = self.nearest_surviving_element_ancestor(node_id);
            self.hover_node_is_text = false;
        }
        if self.hover_hit_node_id == Some(node_id) {
            self.hover_hit_node_id = None;
        }
        if self.active_node_id == Some(node_id) {
            self.active_node_id = self.nearest_surviving_element_ancestor(node_id);
        }
        let kb_active_in_removed_subtree = self.keyboard_active_node_id.is_some_and(|kb_id| {
            let mut curr = Some(kb_id);
            while let Some(id) = curr {
                if id == node_id {
                    return true;
                }
                curr = self.get_node(id).and_then(|n| n.parent);
            }
            false
        });
        if kb_active_in_removed_subtree {
            self.clear_keyboard_active_node();
        }
        let focus_in_removed_subtree = self.focus_node_id.is_some_and(|focus_id| {
            let mut curr = Some(focus_id);
            while let Some(id) = curr {
                if id == node_id {
                    return true;
                }
                curr = self.get_node(id).and_then(|n| n.parent);
            }
            false
        });
        if let Some(focus_id) = self.focus_node_id.filter(|_| focus_in_removed_subtree) {
            let shell_provider = self.shell_provider.clone();
            self.nodes[focus_id].blur(shell_provider);
            let mut maybe_id = Some(focus_id);
            while let Some(node) = maybe_id.and_then(|id| self.get_node(id)) {
                let id = node.id;
                maybe_id = node.parent;
                if node.flags.is_in_document() {
                    self.snapshot_node_and(id, ElementState::FOCUS_WITHIN, |node| {
                        node.remove_focus_within()
                    });
                } else {
                    self.nodes[id].remove_focus_within();
                }
            }
            self.focus_node_id = None;
            self.has_changes = true;
        }
        if self.mousedown_node_id == Some(node_id) {
            self.mousedown_node_id = None;
        }
        if self.text_selection.anchor.node_or_parent == Some(node_id)
            || self.text_selection.focus.node_or_parent == Some(node_id)
        {
            self.text_selection.clear();
        }
        if self
            .hovered_scrollbar
            .is_some_and(|scrollbar| scrollbar.node_id == node_id)
        {
            self.hovered_scrollbar = None;
        }
        let drag_references_node = match &self.drag_mode {
            DragMode::Panning(state) => state.target == node_id,
            DragMode::ScrollbarDrag(state) => state.scrollbar.node_id == node_id,
            DragMode::Selecting | DragMode::None => false,
        };
        if drag_references_node {
            self.drag_mode = DragMode::None;
        }
        self.pending_pointer_capture_targets
            .retain(|_, pending| pending.target != node_id);
        for target in self.pointer_capture_targets.values_mut() {
            if matches!(*target, CaptureTarget::Connected { target, .. } if target == node_id) {
                *target = CaptureTarget::DisconnectedPendingLostCapture;
            }
        }
        let surviving_ancestor = self.nearest_surviving_element_ancestor(node_id);
        for record in self.active_pointers.values_mut() {
            if record.element_under_pointer == Some(node_id) {
                record.element_under_pointer = surviving_ancestor;
            }
        }
        self.scrollbar_activity.remove(&node_id);
    }

    pub(crate) fn drop_node_ignoring_parent(&mut self, node_id: NodeId) -> Option<Node> {
        self.drop_node_ignoring_parent_with(node_id, &mut |_| {})
    }

    /// Like [`Self::drop_node_ignoring_parent`], but calls `on_drop` with the id of
    /// every dropped node (the node itself and all of its descendants).
    pub(crate) fn drop_node_ignoring_parent_with(
        &mut self,
        node_id: NodeId,
        on_drop: &mut dyn FnMut(NodeId),
    ) -> Option<Node> {
        let mut node = self.remove_node_from_tree(node_id);
        if let Some(node) = &mut node {
            on_drop(node_id);
            if let Some(before) = node.before() {
                self.drop_node_ignoring_parent_with(before, on_drop);
            }
            if let Some(after) = node.after() {
                self.drop_node_ignoring_parent_with(after, on_drop);
            }

            for &child in &node.children {
                self.drop_node_ignoring_parent_with(child, on_drop);
            }

            // Anonymous blocks live only in the slab, so deallocate the ones this
            // node owns rather than leaking them.
            for &anon_id in &node.anonymous_blocks {
                self.deallocate_anonymous_block(anon_id);
            }
        }
        node
    }

    /// Deallocate an anonymous block created in a previous construction
    /// round, along with any anonymous blocks nested within it.
    pub(crate) fn deallocate_anonymous_block(&mut self, anon_id: NodeId) {
        // The block may already have been removed from the slab (e.g. a
        // whitespace-only anonymous block dropped during construction).
        if !self.nodes.contains_key(anon_id) {
            return;
        }

        // Free any anonymous blocks that this block owns before removing it.
        let nested = std::mem::take(&mut self.nodes[anon_id].anonymous_blocks);
        for nested_id in nested {
            self.deallocate_anonymous_block(nested_id);
        }

        self.remove_node_from_tree(anon_id);
    }

    /// Whether the DOM or focus changed since the last [`Self::clear_changes`]
    /// (i.e. whether the accessibility tree is out of date)
    pub fn has_changes(&self) -> bool {
        self.has_changes
    }

    /// Mark the accessibility tree as up to date
    pub fn clear_changes(&mut self) {
        self.has_changes = false;
    }

    pub fn create_text_node(&mut self, text: &str) -> NodeId {
        let content = text.to_string();
        let data = NodeData::Text(TextNodeData::new(content));
        self.create_node(data)
    }

    pub fn deep_clone_node(&mut self, node_id: NodeId) -> NodeId {
        // Load existing node
        let node = &self.nodes[node_id];
        let mut data = node.data.clone();

        match &mut data {
            NodeData::Element(elem) | NodeData::AnonymousBlock(elem) => {
                if let Some(arc) = elem.style_attribute.as_mut() {
                    let read_guard = self.guard().read();
                    let block = arc.read_with(&read_guard);
                    *arc = ServoArc::new(self.guard().wrap(block.clone()));
                }
            }
            _ => {}
        }

        let children = node.children.clone();

        // Create new node
        let new_node_id = self.create_node(data);

        // Recursively clone children
        let new_children: ThinVec<NodeId> = children
            .into_iter()
            .map(|child_id| self.deep_clone_node(child_id))
            .collect();
        for &child_id in &new_children {
            self.nodes[child_id].parent = Some(new_node_id);
        }
        self.nodes[new_node_id].children = new_children;

        new_node_id
    }

    pub(crate) fn remove_and_drop_pe(&mut self, node_id: NodeId) -> Option<Node> {
        fn remove_pe_ignoring_parent(doc: &mut BaseDocument, node_id: NodeId) -> Option<Node> {
            let mut node = doc.remove_node_from_tree(node_id);
            if let Some(node) = &mut node {
                for &child in &node.children {
                    remove_pe_ignoring_parent(doc, child);
                }
                for &anon_id in &node.anonymous_blocks {
                    doc.deallocate_anonymous_block(anon_id);
                }
            }
            node
        }

        let node = remove_pe_ignoring_parent(self, node_id);

        // Update child_idx values
        if let Some(parent_id) = node.as_ref().and_then(|node| node.parent) {
            let parent = &mut self.nodes[parent_id];
            parent.children.retain(|id| *id != node_id);
        }

        node
    }

    pub(crate) fn resolve_url(&self, raw: &str) -> Option<url::Url> {
        #[cfg(windows)]
        {
            let path = std::path::Path::new(raw);
            if path.is_absolute() {
                if let Ok(file_url) = url::Url::from_file_path(path) {
                    return Some(file_url);
                }
            }
        }
        #[cfg(not(windows))]
        {
            let path = std::path::Path::new(raw);
            if path.is_absolute() && self.url.scheme() == "file" {
                if let Ok(file_url) = url::Url::from_file_path(path) {
                    return Some(file_url);
                }
            }
        }

        self.url.resolve_relative(raw)
    }

    pub fn print_tree(&self) {
        crate::util::walk_tree(0, self.root_node());
    }

    pub fn print_subtree(&self, node_id: NodeId) {
        crate::util::walk_tree(0, &self.nodes[node_id]);
    }

    pub fn reload_resource_by_href(&mut self, href_to_reload: &str) {
        for &node_id in self.nodes_to_stylesheet.keys() {
            let node = &self.nodes[node_id];
            let Some(element) = node.element_data() else {
                continue;
            };

            if element.name.local == local_name!("link") {
                if let Some(href) = element.attr(local_name!("href")) {
                    // println!("Node {node_id} {href} {href_to_reload} {} {}", resolved_href.as_str(), resolved_href.as_str() == url_to_reload);
                    if href == href_to_reload
                        && let Some(resolved_href) = self.resolve_url(href)
                    {
                        self.net_provider.fetch(
                            self.id(),
                            self.build_request(resolved_href.clone()),
                            ResourceHandler::boxed(
                                self.tx.clone(),
                                self.id,
                                Some(node_id),
                                self.shell_provider.clone(),
                                StylesheetHandler {
                                    source_url: resolved_href,
                                    guard: self.guard.clone(),
                                    net_provider: self.net_provider.clone(),
                                    abort_signal: self.abort_signal.clone(),
                                },
                            ),
                        );
                    }
                }
            }
        }
    }

    pub fn process_style_element(&mut self, target_id: NodeId) {
        let css = self.nodes[target_id].text_content();
        let css = html_escape::decode_html_entities(&css);
        let sheet = self.make_stylesheet(&css, Origin::Author);
        self.add_stylesheet_for_node(sheet, target_id);
    }

    pub fn remove_user_agent_stylesheet(&mut self, sheet: &UserAgentStylesheet) {
        if self.useragent_stylesheets().any(|s| *s == sheet.0) {
            self.stylist
                .remove_stylesheet(sheet.0.clone(), &self.guard.read());
        }
    }

    /// The document's base URL
    pub fn url(&self) -> &url::Url {
        &self.url
    }

    /// Iterate over the author stylesheets (from `<style>` and `<link>` nodes)
    /// currently associated with this document
    pub fn author_stylesheets(&self) -> impl Iterator<Item = &DocumentStyleSheet> {
        self.nodes_to_stylesheet.values()
    }

    /// Iterate over the user-agent stylesheets currently associated with this document
    pub fn useragent_stylesheets(&self) -> impl Iterator<Item = &DocumentStyleSheet> {
        (0..self.stylist.sheet_count(Origin::UserAgent))
            .filter_map(|i| self.stylist.sheet_at(Origin::UserAgent, i))
    }

    /// Append a user-agent stylesheet. Adding a sheet that is already present is a no-op.
    pub fn add_user_agent_stylesheet(&mut self, sheet: UserAgentStylesheet) {
        if self.useragent_stylesheets().any(|s| *s == sheet.0) {
            return;
        }
        self.stylist.append_stylesheet(sheet.0, &self.guard.read());
    }

    pub fn make_stylesheet(&self, css: impl AsRef<str>, origin: Origin) -> DocumentStyleSheet {
        let data = Stylesheet::from_str(
            css.as_ref(),
            self.url.url_extra_data(),
            origin,
            ServoArc::new(self.guard.wrap(MediaList::empty())),
            self.guard.clone(),
            Some(&StylesheetLoader {
                tx: self.tx.clone(),
                doc_id: self.id,
                net_provider: self.net_provider.clone(),
                shell_provider: self.shell_provider.clone(),
                abort_signal: self.abort_signal.clone(),
                import_depth: 0,
            }),
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        );

        DocumentStyleSheet(ServoArc::new(data))
    }

    pub fn upsert_stylesheet_for_node(&mut self, node_id: NodeId) {
        let raw_styles = self.nodes[node_id].text_content();
        let sheet = self.make_stylesheet(raw_styles, Origin::Author);
        self.add_stylesheet_for_node(sheet, node_id);
    }

    pub fn add_stylesheet_for_node(&mut self, stylesheet: DocumentStyleSheet, node_id: NodeId) {
        let old = self.nodes_to_stylesheet.insert(node_id, stylesheet.clone());
        self.stylesheet_generation += 1;

        if let Some(old) = old {
            self.stylist.remove_stylesheet(old, &self.guard.read())
        }

        // Fetch @font-face fonts
        crate::net::fetch_font_face(
            self.tx.clone(),
            self.id,
            Some(node_id),
            &stylesheet.0,
            &self.net_provider,
            &self.shell_provider,
            &self.guard.read(),
            self.abort_signal.as_ref(),
        );

        // Store data on element
        let element = &mut self.nodes[node_id].element_data_mut().unwrap();
        element.special_data = SpecialElementData::Stylesheet(stylesheet.clone());

        // TODO: Nodes could potentially get reused so ordering by node_id might be wrong.
        let insertion_point = self
            .nodes_to_stylesheet
            .range((Bound::Excluded(node_id), Bound::Unbounded))
            .next()
            .map(|(_, sheet)| sheet);

        if let Some(insertion_point) = insertion_point {
            self.stylist.insert_stylesheet_before(
                stylesheet,
                insertion_point.clone(),
                &self.guard.read(),
            )
        } else {
            self.stylist
                .append_stylesheet(stylesheet, &self.guard.read())
        }
    }

    pub fn handle_messages(&mut self) {
        // Remove event Reciever from the Document so that we can process events
        // without holding a borrow to the Document
        let rx = self.rx.take().unwrap();

        while let Ok(msg) = rx.try_recv() {
            self.handle_message(msg);
        }

        // Put Reciever back
        self.rx = Some(rx);
    }

    pub fn handle_message(&mut self, msg: DocumentEvent) {
        match msg {
            DocumentEvent::ResourceLoad(resource) => self.load_resource(resource),
            DocumentEvent::NavigateIframe { node_id, url } => self.navigate_iframe(node_id, url),
        }
    }

    /// Whether the Document has pending requests for "critical" resources (that should block rendering)
    pub fn has_pending_critical_resources(&self) -> bool {
        !self.pending_critical_resources.is_empty()
    }

    pub fn load_resource(&mut self, res: ResourceLoadResponse) {
        let is_ok = res.result.is_ok();
        let was_pending = self.pending_critical_resources.remove(&res.request_id);

        blitz_traits::probe!(
            "blitz-dom::load_resource",
            "request_id={}, resolved_url={:?}, is_ok={}, was_pending={}, remaining_pending={}",
            res.request_id,
            res.resolved_url,
            is_ok,
            was_pending,
            self.pending_critical_resources.len()
        );

        let resource = match res.result {
            Ok(resource) => resource,
            Err(err) => {
                if let Some(url) = res.resolved_url.as_ref() {
                    let waiting_nodes = self.pending_images.remove(url).unwrap_or_default();
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        url = url.as_str(),
                        waiting_nodes = waiting_nodes.len(),
                        error = err.as_str(),
                        "Resource load failed"
                    );
                    #[cfg(not(feature = "tracing"))]
                    let _ = (waiting_nodes, err);
                } else {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(error = err.as_str(), "Resource load failed (no url)");
                    #[cfg(not(feature = "tracing"))]
                    let _ = err;
                }
                return;
            }
        };

        match resource {
            Resource::Css(css) => {
                let node_id = res.node_id.unwrap();
                self.add_stylesheet_for_node(css, node_id);
            }
            Resource::ImportedCss(import_rule, sheet) => {
                let mut guard = self.guard.write();
                import_rule.write_with(&mut guard).stylesheet =
                    style::stylesheets::import_rule::ImportSheet::Sheet(sheet);
                drop(guard);
                self.stylist
                    .force_stylesheet_origins_dirty(style::stylesheets::OriginSet::all());
            }
            Resource::Image(_kind, width, height, image_data) => {
                // Create the ImageData and cache it
                let image = ImageData::Raster(RasterImageData::new(width, height, image_data));

                let Some(url) = res.resolved_url.as_ref() else {
                    return;
                };

                self.apply_loaded_image(url, image);
            }
            #[cfg(feature = "svg")]
            Resource::Svg(_kind, svg) => {
                // Create the ImageData and cache it
                let image = ImageData::Svg(svg);

                let Some(url) = res.resolved_url.as_ref() else {
                    return;
                };

                self.apply_loaded_image(url, image);
            }
            Resource::DocumentSrc(html) => {
                let Some(node_id) = res.node_id else {
                    return;
                };
                self.apply_iframe_html(node_id, res.request_id, res.resolved_url, &html);
            }
            Resource::Font(bytes, overrides) => {
                let font = Blob::new(Arc::new(bytes));

                // Build a `FontInfoOverride` from the `@font-face` descriptors
                // captured during stylesheet parsing. Without this, parley
                // reads the family name from the TTF's own metadata, which
                // means CSS `font-family: 'Avenir Book'` won't match a font
                // file that internally identifies as `Avenir 45 Book`.
                let weight_override = overrides.weight.map(parley::fontique::FontWeight::new);
                let info_override = parley::fontique::FontInfoOverride {
                    family_name: overrides.family_name.as_deref(),
                    weight: weight_override,
                    style: overrides.style,
                    ..Default::default()
                };

                // TODO: Investigate eliminating double-box
                let mut global_font_ctx = self.font_ctx.lock().unwrap();
                global_font_ctx
                    .collection
                    .register_fonts(font.clone(), Some(info_override));

                #[cfg(feature = "parallel-construct")]
                {
                    rayon::broadcast(|_ctx| {
                        let mut font_ctx = self
                            .thread_font_contexts
                            .get_or(|| RefCell::new(Box::new(global_font_ctx.clone())))
                            .borrow_mut();
                        font_ctx
                            .collection
                            .register_fonts(font.clone(), Some(info_override));
                    });
                }
                drop(global_font_ctx);

                // TODO: see if we can only invalidate if resolved fonts may have changed
                self.invalidate_inline_contexts();
            }
            Resource::None => {
                // Do nothing
            }
        }
    }

    /// Cache a loaded image and apply it to all nodes waiting on it
    /// (`<img>` elements, `background-image` layers and `mask-image` layers).
    fn apply_loaded_image(&mut self, url: &str, image: ImageData) {
        // Get all nodes waiting for this image
        let waiting_nodes = self.pending_images.remove(url).unwrap_or_default();

        #[cfg(feature = "tracing")]
        tracing::info!(
            "Image {url} loaded, applying to {} nodes",
            waiting_nodes.len()
        );

        // Cache the image
        self.image_cache.insert(url.to_string(), image.clone());

        // Apply to all waiting nodes
        for (node_id, image_type) in waiting_nodes {
            let Some(node) = self.get_node_mut(node_id) else {
                continue;
            };

            match image_type {
                ImageType::Image => {
                    let element = node.element_data_mut().unwrap();
                    #[cfg(feature = "custom-widget")]
                    if element.custom_widget_data().is_some() {
                        continue;
                    }
                    element.special_data = SpecialElementData::Image(Box::new(image.clone()));

                    // Clear layout cache
                    node.clear_layout_cache();
                    node.insert_damage(ALL_DAMAGE);
                }
                ImageType::Background(idx) | ImageType::Mask(idx) => {
                    let layer_image = node.element_data_mut().and_then(|el| {
                        let images = match image_type {
                            ImageType::Background(_) => &mut el.background_images,
                            ImageType::Mask(_) => &mut el.mask_images,
                            ImageType::Image => unreachable!(),
                        };
                        images.get_mut(idx)
                    });
                    if let Some(Some(layer_image)) = layer_image {
                        layer_image.status = Status::Ok;
                        layer_image.image = image.clone();
                    }
                }
            }
        }
    }

    /// Snapshot the node's pre-mutation state (element state and attributes) ahead of
    /// an attribute mutation, so that the next style traversal can diff selector matches
    /// then-vs-now and invalidate the affected elements. `attr` is the attribute about to
    /// change; it is recorded as changed even if the element does not have it yet.
    pub fn snapshot_node(&mut self, node_id: NodeId, attr: &LocalName) {
        self.snapshot_node_impl(node_id, Some(attr))
    }

    /// Snapshot only the node's pre-change [`ElementState`] ahead of a state change
    /// (hover/focus/active/etc). Cheaper than [`Self::snapshot_node`] as it does not
    /// copy attributes or trigger attribute/class/id invalidation work.
    pub fn snapshot_node_state_only(&mut self, node_id: NodeId) {
        self.snapshot_node_impl(node_id, None)
    }

    fn snapshot_node_impl(&mut self, node_id: NodeId, changed_attr: Option<&LocalName>) {
        let node = &mut self.nodes[node_id];

        // Do not snapshot nodes that have never been styled. A snapshot records an element's
        // pre-mutation state so a restyle can diff selector matches then-vs-now. An element
        // that has never been styled has no "then" to diff against. Snapshotting it anyway
        // makes Stylo's invalidation unwrap its (absent) primary style and panic.
        let has_been_styled = node.primary_styles().is_some();
        if !has_been_styled {
            return;
        }

        let opaque_node_id = node.opaque();
        node.set_has_snapshot(true);
        node.snapshot_handled()
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // A snapshot records the element's state/attributes as they were *before the first
        // mutation* since the last style flush (matching Gecko's ServoElementSnapshot
        // semantics): state is captured at most once, and attributes are captured at most
        // once, but a state-only snapshot is upgraded to also capture attributes if an
        // attribute mutation follows.
        let needs_attrs = changed_attr.is_some()
            && self
                .snapshots
                .get_mut(&opaque_node_id)
                .is_none_or(|snapshot| snapshot.attrs.is_none());

        let (attrs, changed_attrs) = if needs_attrs {
            let node = &self.nodes[node_id];
            let attrs: Option<Vec<_>> = node.attrs().map(|attrs| {
                attrs
                    .iter()
                    .map(|attr| {
                        let ident = AttrIdentifier {
                            local_name: GenericAtomIdent(attr.name.local.clone()),
                            name: GenericAtomIdent(attr.name.local.clone()),
                            namespace: GenericAtomIdent(attr.name.ns.clone()),
                            prefix: None,
                        };

                        let value = if attr.name.local == local_name!("id") {
                            AttrValue::Atom(Atom::from(&*attr.value))
                        } else if attr.name.local == local_name!("class") {
                            let classes = attr
                                .value
                                .split_ascii_whitespace()
                                .map(Atom::from)
                                .collect();
                            AttrValue::TokenList(OnceLock::from(attr.value.clone()), classes)
                        } else {
                            AttrValue::String(attr.value.clone())
                        };

                        (ident, value)
                    })
                    .collect()
            });

            let mut changed_attrs: Vec<_> = attrs
                .as_ref()
                .map(|attrs| attrs.iter().map(|attr| attr.0.name.clone()).collect())
                .unwrap_or_default();
            if let Some(attr) = changed_attr {
                if !changed_attrs.iter().any(|name| name.0 == *attr) {
                    changed_attrs.push(GenericAtomIdent(attr.clone()));
                }
            }

            (attrs, changed_attrs)
        } else {
            (None, Vec::new())
        };

        if let Some(snapshot) = self.snapshots.get_mut(&opaque_node_id) {
            // The existing snapshot's state is preserved: it records the state before
            // the *first* change since the last style flush.
            if needs_attrs {
                snapshot.attrs = attrs;
                snapshot.changed_attrs = changed_attrs;
                snapshot.class_changed = true;
                snapshot.id_changed = true;
                snapshot.other_attributes_changed = true;
            } else if let Some(attr) = changed_attr {
                if !snapshot.changed_attrs.iter().any(|name| name.0 == *attr) {
                    snapshot.changed_attrs.push(GenericAtomIdent(attr.clone()));
                }
            }
        } else {
            self.snapshots.insert(
                opaque_node_id,
                ServoElementSnapshot {
                    state: Some(*self.nodes[node_id].element_state()),
                    attrs,
                    changed_attrs,
                    class_changed: needs_attrs,
                    id_changed: needs_attrs,
                    other_attributes_changed: needs_attrs,
                },
            );
        }
    }

    /// Returns whether any style rule depends on any of the given [`ElementState`] bits.
    /// If not, changing those bits cannot affect styling and snapshotting can be skipped.
    pub fn style_depends_on_state(&self, state: ElementState) -> bool {
        self.stylist.iter_origins().any(|(data, _)| {
            data.has_state_dependency(state) || data.has_nth_of_state_dependency(state)
        })
    }

    /// Apply a state change (hover/focus/active/etc affecting the given [`ElementState`]
    /// bits) to a node, taking a state-only snapshot beforehand if any style rule
    /// depends on those bits.
    pub fn snapshot_node_and(
        &mut self,
        node_id: NodeId,
        state: ElementState,
        cb: impl FnOnce(&mut Node),
    ) {
        if self.style_depends_on_state(state) {
            self.snapshot_node_state_only(node_id);
        }
        cb(&mut self.nodes[node_id]);
    }

    // Takes (x, y) co-ordinates (relative to the )
    pub fn hit(&self, x: f32, y: f32) -> Option<HitResult> {
        self.hit_with_scrollbar(x, y).0
    }

    /// The topmost element at viewport coordinates (x, y), or `None` if the point
    /// is outside the viewport. Anonymous boxes and text nodes are resolved to
    /// their nearest element; a point over the background hits the root element.
    ///
    /// This implements the hit-testing semantics of `document.elementFromPoint()`.
    /// Hit testing consults layout, so [`resolve`](Self::resolve) should be called
    /// before this method to ensure layout is up to date.
    pub fn element_from_point(&self, x: f32, y: f32) -> Option<NodeId> {
        let viewport = self.viewport();
        let scale = viewport.scale();
        let (viewport_width, viewport_height) = (
            viewport.window_size.0 as f32 / scale,
            viewport.window_size.1 as f32 / scale,
        );
        if x < 0.0 || y < 0.0 || x > viewport_width || y > viewport_height {
            return None;
        }

        let Some(hit) = self.hit(x, y) else {
            // The point is within the viewport but over no box: the root element
            // (which covers the viewport in an HTML document) is the hit target
            return self.try_root_element().map(|root| root.id);
        };

        // Resolve anonymous boxes / pseudo-elements, then text nodes, to elements
        let mut node_id = self.nearest_non_anonymous_ancestor(hit.node_id)?;
        loop {
            let node = self.get_node(node_id)?;
            if node.is_element() {
                return Some(node_id);
            }
            node_id = node.parent?;
        }
    }

    /// All elements at viewport coordinates (x, y), from topmost to bottommost,
    /// as for `document.elementsFromPoint()`.
    ///
    /// The spec's paint-order list is approximated with the hit element followed
    /// by its ancestor elements (which is correct for non-overlapping content).
    /// As with [`element_from_point`](Self::element_from_point), layout should be
    /// resolved before calling this method.
    pub fn elements_from_point(&self, x: f32, y: f32) -> Vec<NodeId> {
        let mut element_ids = Vec::new();
        let mut current = self.element_from_point(x, y);
        while let Some(node_id) = current {
            element_ids.push(node_id);
            current = self
                .get_node(node_id)
                .and_then(|node| node.parent)
                .filter(|parent_id| {
                    self.get_node(*parent_id)
                        .is_some_and(|node| node.is_element())
                });
        }
        element_ids
    }

    /// Walk up the tree to the nearest DOM node whose id is stable across
    /// box-tree reconstruction, so canonicalized interaction state never goes
    /// stale.
    ///
    /// Layout-generated nodes (anonymous blocks and `::before`/`::after`
    /// pseudo-elements, both stored as anonymous blocks) get new ids on every
    /// reconstruction, so we skip any anonymous node *and* a non-anonymous node
    /// whose parent is anonymous (the pseudo's text content). The first
    /// non-anonymous node with a non-anonymous parent is a real DOM node; the
    /// root element's `Document` parent guarantees termination.
    ///
    /// Returns `None` if `node_id` (or an ancestor) no longer exists.
    pub fn nearest_non_anonymous_ancestor(&self, node_id: NodeId) -> Option<NodeId> {
        // Recurse up the tree keeping a window of the current node and its
        // parent, advancing one step per iteration so each node is looked up
        // exactly once.
        let mut node = self.get_node(node_id)?;
        loop {
            let parent = match node.parent {
                Some(parent_id) => self.get_node(parent_id)?,
                None => return Some(node.id),
            };
            if !node.is_anonymous() && !parent.is_anonymous() {
                return Some(node.id);
            }
            node = parent;
        }
    }

    pub fn focus_next_node(&mut self) -> Option<NodeId> {
        let id = if let Some(focussed_node_id) = self.get_focussed_node_id() {
            self.next_node(&self.nodes[focussed_node_id], |node| {
                self.is_in_tab_order(node)
            })?
        } else {
            let root = self.root_node_id;
            self.next_node(&self.nodes[root], |node| self.is_in_tab_order(node))?
        };
        self.set_focus_to_with_source(id, FocusSource::Keyboard);
        Some(id)
    }

    /// Move focus to the previous focussable node in the document
    pub fn focus_prev_node(&mut self) -> Option<NodeId> {
        let id = if let Some(focussed_node_id) = self.get_focussed_node_id() {
            self.prev_node(&self.nodes[focussed_node_id], |node| {
                self.is_in_tab_order(node)
            })?
        } else {
            let root = self.root_node_id;
            self.prev_node(&self.nodes[root], |node| self.is_in_tab_order(node))?
        };
        self.set_focus_to_with_source(id, FocusSource::Keyboard);
        Some(id)
    }

    /// HTML §4.10.18.5: Whether `node_id` is a form control (`button`, `input`,
    /// `select`, `textarea`) that is disabled either via its own `disabled`
    /// attribute or via an ancestor `<fieldset disabled>` (unless `node_id` is
    /// a descendant of that `<fieldset>`'s first `<legend>` element child).
    pub fn is_form_control_disabled(&self, node_id: NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        let Some(el) = node.element_data() else {
            return false;
        };
        if !el.can_be_disabled() {
            return false;
        }
        if el.has_attr(local_name!("disabled")) {
            return true;
        }

        let mut child_id = node_id;
        while let Some(parent_id) = self.get_node(child_id).and_then(|n| n.parent) {
            let Some(parent) = self.get_node(parent_id) else {
                break;
            };
            if parent
                .data
                .is_element_with_tag_name(&local_name!("fieldset"))
                && parent.data.has_attr(local_name!("disabled"))
            {
                let first_legend_id = parent.children.iter().copied().find(|&cid| {
                    self.get_node(cid)
                        .is_some_and(|n| n.data.is_element_with_tag_name(&local_name!("legend")))
                });
                if Some(child_id) != first_legend_id {
                    return true;
                }
            }
            child_id = parent_id;
        }

        false
    }

    /// Whether `node_id` is the controlling `<summary>` of a `<details>` parent
    /// (i.e. parent is `<details>` and `node_id` is its first `<summary>` element child).
    pub fn is_controlling_summary(&self, node_id: NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        if !node.data.is_element_with_tag_name(&local_name!("summary")) {
            return false;
        }
        let Some(parent_id) = node.parent else {
            return false;
        };
        let Some(parent) = self.get_node(parent_id) else {
            return false;
        };
        parent
            .data
            .is_element_with_tag_name(&local_name!("details"))
            && parent.children.iter().copied().find(|&child_id| {
                self.get_node(child_id)
                    .is_some_and(|n| n.data.is_element_with_tag_name(&local_name!("summary")))
            }) == Some(node_id)
    }

    pub(crate) fn is_enter_non_link_activatable_element(&self, node_id: NodeId) -> bool {
        let Some(el) = self.get_node(node_id).and_then(|n| n.element_data()) else {
            return false;
        };
        match el.name.local {
            local_name!("button") => true,
            local_name!("input") => el.is_keyboard_clickable_input(),
            local_name!("summary") => self.is_controlling_summary(node_id),
            _ => false,
        }
    }

    pub(crate) fn is_space_activatable_element(&self, node_id: NodeId) -> bool {
        let Some(el) = self.get_node(node_id).and_then(|n| n.element_data()) else {
            return false;
        };
        match el.name.local {
            local_name!("button") => true,
            local_name!("input") => {
                el.is_keyboard_clickable_input() || el.is_checkbox_input() || el.is_radio_input()
            }
            local_name!("summary") => self.is_controlling_summary(node_id),
            _ => false,
        }
    }

    pub(crate) fn is_node_displayed(&self, node_id: NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        if !node.flags.is_in_document() {
            return false;
        }
        // UA stylesheet (`default.css`) sets `area { display: none !important; }` so `<area>`
        // generates no CSS box of its own while remaining focusable/interactive when its
        // ancestors are displayed.
        let mut maybe_ancestor = if node.data.is_element_with_tag_name(&local_name!("area")) {
            node.parent_node()
        } else {
            Some(node)
        };
        while let Some(ancestor) = maybe_ancestor {
            if ancestor
                .primary_styles()
                .is_some_and(|styles| styles.clone_display().is_none())
            {
                return false;
            }
            maybe_ancestor = ancestor.parent_node();
        }
        true
    }

    fn is_in_tab_order(&self, node: &Node) -> bool {
        if !node.is_focussable() || self.is_form_control_disabled(node.id) {
            return false;
        }
        if node.data.is_element_with_tag_name(&local_name!("summary"))
            && !node.data.has_attr(local_name!("tabindex"))
            && !self.is_controlling_summary(node.id)
        {
            return false;
        }
        // Focusability is cached from attributes. A display:none ancestor
        // suppresses the whole subtree even when the control's own cached
        // focusability (or styles from an earlier layout) is unchanged.
        self.is_node_displayed(node.id)
    }

    /// Clear the focussed node
    pub fn clear_focus(&mut self) {
        self.clear_keyboard_active_node();
        if let Some(id) = self.focus_node_id {
            let shell_provider = self.shell_provider.clone();
            self.snapshot_node_and(id, ElementState::FOCUS | ElementState::FOCUSRING, |node| {
                node.blur(shell_provider)
            });
            for &ancestor in self.node_element_ancestors(id).iter() {
                self.snapshot_node_and(ancestor, ElementState::FOCUS_WITHIN, |node| {
                    node.remove_focus_within()
                });
            }
            self.focus_node_id = None;
            self.has_changes = true;
            self.shell_provider.request_redraw();
        }
    }

    pub fn set_mousedown_node_id(&mut self, node_id: Option<NodeId>) {
        self.mousedown_node_id = node_id.and_then(|id| self.nearest_non_anonymous_ancestor(id));
    }
    pub fn set_focus_to(&mut self, focus_node_id: NodeId) -> bool {
        self.set_focus_to_with_source(focus_node_id, FocusSource::Script)
    }

    pub fn set_focus_to_with_source(&mut self, focus_node_id: NodeId, source: FocusSource) -> bool {
        let Some(focus_node_id) = self.nearest_non_anonymous_ancestor(focus_node_id) else {
            return false;
        };
        let focus_visible = match source {
            FocusSource::Keyboard | FocusSource::Script => true,
            // Pointer-initiated focus only shows a focus ring on elements which
            // support keyboard input (per the :focus-visible heuristics)
            FocusSource::Pointer => self.nodes[focus_node_id]
                .element_data()
                .is_some_and(|el| el.text_input_data().is_some()),
        };
        if Some(focus_node_id) == self.focus_node_id {
            // Update the focus ring visibility to reflect the latest focus source
            let has_ring = self.nodes[focus_node_id]
                .element_state()
                .contains(ElementState::FOCUSRING);
            if has_ring != focus_visible {
                self.snapshot_node_and(focus_node_id, ElementState::FOCUSRING, |node| {
                    if let Some(data) = node.element_data_mut() {
                        data.element_state
                            .set(ElementState::FOCUSRING, focus_visible);
                    }
                    node.mark_ancestors_dirty();
                });
                self.has_changes = true;
                self.shell_provider.request_redraw();
            }
            return false;
        }
        // A node that is not in the document cannot be focused
        if !self.nodes[focus_node_id].flags.is_in_document() {
            return false;
        }

        // Changing focus disarms any active Space key press
        self.clear_keyboard_active_node();

        #[cfg(feature = "tracing")]
        tracing::info!("Focussed node {focus_node_id}");

        let shell_provider = self.shell_provider.clone();

        // Remove focus from the old node
        if let Some(id) = self.focus_node_id {
            self.snapshot_node_and(id, ElementState::FOCUS | ElementState::FOCUSRING, |node| {
                node.blur(shell_provider.clone())
            });
        }

        // Focus the new node
        self.snapshot_node_and(
            focus_node_id,
            ElementState::FOCUS | ElementState::FOCUSRING,
            |node| node.focus(shell_provider, focus_visible),
        );

        // Update :focus-within on the ancestor chains (shared prefix stays)
        let old_node_path = self.maybe_node_element_ancestors(self.focus_node_id);
        let new_node_path = self.node_element_ancestors(focus_node_id);
        let same_count = old_node_path
            .iter()
            .zip(&new_node_path)
            .take_while(|(o, n)| o == n)
            .count();
        for &id in old_node_path.iter().skip(same_count) {
            self.snapshot_node_and(id, ElementState::FOCUS_WITHIN, |node| {
                node.remove_focus_within()
            });
        }
        for &id in new_node_path.iter().skip(same_count) {
            self.snapshot_node_and(id, ElementState::FOCUS_WITHIN, |node| {
                node.add_focus_within()
            });
        }

        self.focus_node_id = Some(focus_node_id);
        self.has_changes = true;

        // The focus is styled (:focus, :focus-visible, :focus-within), so
        // moving it changes what is on screen. Nothing else asks for the
        // frame: a keyboard focus change touches no layout and no content.
        self.shell_provider.request_redraw();

        true
    }

    pub fn active_node(&mut self) -> bool {
        let Some(hover_node_id) = self.get_hover_node_id() else {
            return false;
        };

        if let Some(active_node_id) = self.active_node_id {
            if active_node_id == hover_node_id {
                return true;
            }
            self.unactive_node();
        }

        // hover_node_id is canonicalized when stored, so this always holds.
        debug_assert!(
            self.get_node(hover_node_id)
                .is_some_and(|node| !node.is_anonymous()),
            "interaction state must reference DOM nodes, not layout-generated nodes"
        );
        let active_node_id = Some(hover_node_id);

        let node_path = self.maybe_node_element_ancestors(active_node_id);
        for &id in node_path.iter() {
            self.snapshot_node_and(id, ElementState::ACTIVE, |node| node.active());
        }

        self.active_node_id = active_node_id;

        true
    }

    pub fn unactive_node(&mut self) -> bool {
        let Some(active_node_id) = self.active_node_id.take() else {
            return false;
        };

        let kb_path = self.maybe_node_element_ancestors(self.keyboard_active_node_id);
        let node_path = self.maybe_node_element_ancestors(Some(active_node_id));
        for &id in node_path.iter() {
            if !kb_path.contains(&id) {
                self.snapshot_node_and(id, ElementState::ACTIVE, |node| node.unactive());
            }
        }

        true
    }

    pub fn get_keyboard_active_node_id(&self) -> Option<NodeId> {
        self.keyboard_active_node_id
    }

    pub fn set_keyboard_active_node(&mut self, node_id: NodeId) -> bool {
        let Some(node_id) = self.nearest_non_anonymous_ancestor(node_id) else {
            return false;
        };
        if !self.nodes[node_id].flags.is_in_document() {
            return false;
        }
        if self.keyboard_active_node_id == Some(node_id) {
            return false;
        }
        self.clear_keyboard_active_node();

        let ptr_path = self.maybe_node_element_ancestors(self.active_node_id);
        let node_path = self.node_element_ancestors(node_id);
        for &id in node_path.iter() {
            if !ptr_path.contains(&id) {
                self.snapshot_node_and(id, ElementState::ACTIVE, |node| node.active());
            }
        }

        self.keyboard_active_node_id = Some(node_id);
        self.has_changes = true;
        self.shell_provider.request_redraw();
        true
    }

    pub fn take_keyboard_active_node(&mut self) -> Option<NodeId> {
        let node_id = self.keyboard_active_node_id.take()?;
        if self.nodes.contains_key(node_id) {
            let ptr_path = self.maybe_node_element_ancestors(self.active_node_id);
            let mut maybe_id = Some(node_id);
            while let Some(node) = maybe_id.and_then(|id| self.get_node(id)) {
                if !node.is_element() {
                    break;
                }
                let id = node.id;
                maybe_id = node.parent;
                if !ptr_path.contains(&id) {
                    if node.flags.is_in_document() {
                        self.snapshot_node_and(id, ElementState::ACTIVE, |node| node.unactive());
                    } else {
                        self.nodes[id].unactive();
                    }
                }
            }
            self.has_changes = true;
            self.shell_provider.request_redraw();
        }
        Some(node_id)
    }

    pub fn clear_keyboard_active_node(&mut self) -> bool {
        self.take_keyboard_active_node().is_some()
    }

    /// The scrollbar thumb currently under the pointer, if any.
    pub fn hovered_scrollbar(&self) -> Option<crate::node::ScrollbarRef> {
        self.hovered_scrollbar
    }

    /// The scrollbar thumb currently being dragged, if any.
    pub fn scrollbar_drag_target(&self) -> Option<crate::node::ScrollbarRef> {
        match &self.drag_mode {
            DragMode::ScrollbarDrag(state) => Some(state.scrollbar),
            _ => None,
        }
    }

    /// The current opacity of `node_id`'s overlay scrollbars. They show at
    /// full opacity on scroll and fade out after a delay (Chromium's overlay
    /// timings); the pointer resting on a thumb, or dragging it, holds them
    /// visible.
    pub fn scrollbar_opacity(&self, node_id: NodeId) -> f32 {
        let interacting = |scrollbar: &crate::node::ScrollbarRef| scrollbar.node_id == node_id;
        if self.hovered_scrollbar.as_ref().is_some_and(interacting)
            || self
                .scrollbar_drag_target()
                .as_ref()
                .is_some_and(interacting)
        {
            return 1.0;
        }
        self.scrollbar_activity.get(&node_id).map_or(0.0, |last| {
            crate::node::scrollbar::opacity_at(last.elapsed())
        })
    }

    /// Show `node_id`'s overlay scrollbars at full opacity and restart their
    /// fade-out delay.
    pub(crate) fn show_scrollbars(&mut self, node_id: NodeId) {
        if cfg!(feature = "scrollbars") {
            self.scrollbar_activity.insert(node_id, Instant::now());
        }
    }

    /// Whether any overlay scrollbars are awaiting or animating their
    /// fade-out (so frames must keep rendering until they finish).
    fn scrollbars_animating(&self) -> bool {
        use crate::node::scrollbar::{FADE_DELAY, FADE_DURATION};
        self.scrollbar_activity
            .values()
            .any(|last| last.elapsed() < FADE_DELAY + FADE_DURATION)
    }

    /// [`hit`](Self::hit), also resolving the innermost overlay scrollbar
    /// thumb under the point (shares the traversal, so it costs nothing
    /// extra).
    pub(crate) fn hit_with_scrollbar(
        &self,
        x: f32,
        y: f32,
    ) -> (Option<HitResult>, Option<crate::node::ScrollbarRef>) {
        if self.root_node().first_element_child().is_none() {
            #[cfg(feature = "tracing")]
            tracing::warn!("No DOM - not resolving hit test");
            return (None, None);
        }
        let mut scrollbar = None;
        let hit = self
            .root_element()
            .hit_inner(x, y, self.viewport().scale_f64(), &mut scrollbar);
        (hit, scrollbar)
    }

    pub fn set_hover_to(&mut self, x: f32, y: f32) -> bool {
        // Record the pointer position in client (unscrolled) coordinates so
        // that `refresh_hover` can re-resolve hover state after layout or
        // scroll changes.
        self.last_client_pointer_position = Some(taffy::Point {
            x: x - self.viewport_scroll().x as f32,
            y: y - self.viewport_scroll().y as f32,
        });

        let (hit, hovered_scrollbar) = self.hit_with_scrollbar(x, y);
        // A faded-out thumb is not interactive: pointer moves never fade
        // overlay scrollbars back in (only scrolling shows them).
        let hovered_scrollbar =
            hovered_scrollbar.filter(|scrollbar| self.scrollbar_opacity(scrollbar.node_id) > 0.0);
        // Scrollbar-thumb hover is part of hover state: track it here so a
        // pointer crossing a thumb restyles it even when the hit node (the
        // content under the overlay thumb) is unchanged.
        let scrollbar_changed = hovered_scrollbar != self.hovered_scrollbar;
        if scrollbar_changed {
            // Entering a thumb restores full opacity mid-fade; leaving one
            // restarts the fade-out delay.
            for scrollbar in [self.hovered_scrollbar, hovered_scrollbar]
                .into_iter()
                .flatten()
            {
                self.show_scrollbars(scrollbar.node_id);
            }
        }
        self.hovered_scrollbar = hovered_scrollbar;

        // Store both the precise layout node that was hit (transient: used for
        // cursor/style queries) and its canonical DOM target (persistent: must
        // not reference layout-generated nodes, whose ids die on box-tree
        // reconstruction).
        let hit_node_id = hit.map(|hit| hit.node_id);
        let hover_node_id = hit_node_id.and_then(|id| self.nearest_non_anonymous_ancestor(id));
        let new_is_text = hit.map(|hit| hit.is_text).unwrap_or(false);

        self.hover_hit_node_id = hit_node_id;
        self.hover_node_is_text = new_is_text;

        // Return early if the new node is the same as the already-hovered node
        if hover_node_id == self.hover_node_id {
            return scrollbar_changed;
        }

        let old_node_path = self.maybe_node_element_ancestors(self.hover_node_id);
        let new_node_path = self.maybe_node_element_ancestors(hover_node_id);
        let same_count = old_node_path
            .iter()
            .zip(&new_node_path)
            .take_while(|(o, n)| o == n)
            .count();
        for &id in old_node_path.iter().skip(same_count) {
            self.snapshot_node_and(id, ElementState::HOVER, |node| node.unhover());
        }
        for &id in new_node_path.iter().skip(same_count) {
            self.snapshot_node_and(id, ElementState::HOVER, |node| node.hover());
        }

        self.hover_node_id = hover_node_id;

        // Request redraw
        self.shell_provider.request_redraw();

        true
    }

    pub fn clear_hover(&mut self) -> bool {
        // The pointer is no longer over the document, so stop re-resolving
        // hover state against it.
        self.last_client_pointer_position = None;
        self.hover_hit_node_id = None;

        let Some(hover_node_id) = self.hover_node_id else {
            return false;
        };

        let old_node_path = self.maybe_node_element_ancestors(Some(hover_node_id));
        for &id in old_node_path.iter() {
            self.snapshot_node_and(id, ElementState::HOVER, |node| node.unhover());
        }

        self.hover_node_id = None;
        self.hover_node_is_text = false;

        // Request redraw
        self.shell_provider.request_redraw();

        true
    }

    /// Re-resolve hover state against the current layout using the last known
    /// pointer position.
    ///
    /// TODO: synthesizing pointerenter/pointerleave DOM events for
    /// hover changes caused by layout shifts.
    pub fn refresh_hover(&mut self) -> bool {
        let Some(pos) = self.last_client_pointer_position else {
            return false;
        };
        let x = pos.x + self.viewport_scroll().x as f32;
        let y = pos.y + self.viewport_scroll().y as f32;
        let prev_hover = self.hover_state();
        let changed = if let Some(captured_id) = self.active_primary_capture_node() {
            self.set_hover_to_captured_node(captured_id, x, y)
        } else {
            self.set_hover_to(x, y)
        };
        if self.hover_state() != prev_hover {
            self.update_cursor();
        }
        changed
    }

    pub fn set_pointer_capture(
        &mut self,
        node_id: NodeId,
        pointer_id: i32,
    ) -> Result<(), PointerCaptureError> {
        let Some(record) = self.active_pointers.get(&pointer_id) else {
            return Err(PointerCaptureError::NotFound);
        };
        if !self.is_connected_element(node_id) {
            return Err(PointerCaptureError::InvalidState);
        }
        if record.buttons.is_empty() {
            return Ok(());
        }
        self.pending_pointer_capture_targets.insert(
            pointer_id,
            PendingCapture {
                target: node_id,
                explicit: true,
            },
        );
        Ok(())
    }

    pub fn release_pointer_capture(
        &mut self,
        node_id: NodeId,
        pointer_id: i32,
    ) -> Result<(), PointerCaptureError> {
        if !self.active_pointers.contains_key(&pointer_id) {
            return Err(PointerCaptureError::NotFound);
        }
        if self.has_pointer_capture(node_id, pointer_id) {
            self.pending_pointer_capture_targets.remove(&pointer_id);
        }
        Ok(())
    }

    pub fn has_pointer_capture(&self, node_id: NodeId, pointer_id: i32) -> bool {
        if !self.is_connected_element(node_id) {
            return false;
        }
        self.pending_pointer_capture_targets
            .get(&pointer_id)
            .is_some_and(|pending| pending.target == node_id)
    }

    pub(crate) fn set_implicit_pointer_capture(&mut self, node_id: NodeId, pointer_id: i32) {
        let Some(target) = self.nearest_connected_element(node_id) else {
            return;
        };
        if self
            .active_pointers
            .get(&pointer_id)
            .is_some_and(|record| !record.buttons.is_empty())
        {
            self.pending_pointer_capture_targets.insert(
                pointer_id,
                PendingCapture {
                    target,
                    explicit: false,
                },
            );
        }
    }

    pub(crate) fn is_connected_element(&self, node_id: NodeId) -> bool {
        self.nodes
            .get(node_id)
            .is_some_and(|node| node.is_element() && node.flags.is_in_document())
    }

    pub(crate) fn nearest_connected_element(&self, node_id: NodeId) -> Option<NodeId> {
        let mut current = Some(node_id);
        while let Some(id) = current {
            let node = self.get_node(id)?;
            if node.is_element() && node.flags.is_in_document() {
                return Some(id);
            }
            current = node.parent;
        }
        None
    }

    pub(crate) fn common_element_ancestor(&self, a: NodeId, b: NodeId) -> Option<NodeId> {
        let a = self.nearest_connected_element(a)?;
        let b = self.nearest_connected_element(b)?;
        if a == b {
            return Some(a);
        }
        let a_chain = self.node_element_ancestors(a);
        let b_chain = self.node_element_ancestors(b);
        a_chain
            .iter()
            .zip(&b_chain)
            .take_while(|(x, y)| x == y)
            .last()
            .map(|(&id, _)| id)
    }

    pub(crate) fn active_capture_node_by_dom_id(&self, pointer_id: i32) -> Option<NodeId> {
        match self.pointer_capture_targets.get(&pointer_id) {
            Some(CaptureTarget::Connected { target, .. }) if self.is_connected_element(*target) => {
                Some(*target)
            }
            _ => None,
        }
    }

    pub(crate) fn active_capture_node(&self, pointer_id: BlitzPointerId) -> Option<NodeId> {
        self.active_capture_node_by_dom_id(pointer_id.to_dom_pointer_id())
    }

    pub(crate) fn has_explicit_pointer_capture(&self, pointer_id: BlitzPointerId) -> bool {
        matches!(
            self.pointer_capture_targets.get(&pointer_id.to_dom_pointer_id()),
            Some(CaptureTarget::Connected { target, explicit: true }) if self.is_connected_element(*target)
        )
    }

    pub(crate) fn active_primary_capture_node(&self) -> Option<NodeId> {
        self.active_pointers
            .iter()
            .filter(|(_, record)| record.is_primary)
            .find_map(|(&pointer_id, _)| self.active_capture_node_by_dom_id(pointer_id))
    }

    pub(crate) fn set_hover_to_captured_node(
        &mut self,
        captured_id: NodeId,
        x: f32,
        y: f32,
    ) -> bool {
        self.last_client_pointer_position = Some(taffy::Point {
            x: x - self.viewport_scroll().x as f32,
            y: y - self.viewport_scroll().y as f32,
        });

        let scrollbar_changed = self.hovered_scrollbar.is_some();
        if let Some(scrollbar) = self.hovered_scrollbar.take() {
            self.show_scrollbars(scrollbar.node_id);
        }

        let hover_node_id = self.nearest_connected_element(captured_id);
        self.hover_hit_node_id = hover_node_id;
        self.hover_node_is_text = false;

        if hover_node_id == self.hover_node_id {
            return scrollbar_changed;
        }

        let old_node_path = self.maybe_node_element_ancestors(self.hover_node_id);
        let new_node_path = self.maybe_node_element_ancestors(hover_node_id);
        let same_count = old_node_path
            .iter()
            .zip(&new_node_path)
            .take_while(|(o, n)| o == n)
            .count();
        for &id in old_node_path.iter().skip(same_count) {
            self.snapshot_node_and(id, ElementState::HOVER, |node| node.unhover());
        }
        for &id in new_node_path.iter().skip(same_count) {
            self.snapshot_node_and(id, ElementState::HOVER, |node| node.hover());
        }

        self.hover_node_id = hover_node_id;
        self.shell_provider.request_redraw();

        true
    }

    /// The hover state that determines the cursor: the hovered node, the precise hit
    /// node and whether the hit is text
    pub(crate) fn hover_state(&self) -> (Option<NodeId>, Option<NodeId>, bool) {
        (
            self.hover_node_id,
            self.hover_hit_node_id,
            self.hover_node_is_text,
        )
    }

    /// Send the current cursor to the shell.
    ///
    /// Hover updates don't send the cursor themselves: the cursor of a node hosting a
    /// sub-document is only known once the sub-document has handled the event.
    pub(crate) fn update_cursor(&self) {
        self.shell_provider.set_cursor(self.get_cursor());
    }

    pub fn get_hover_node_id(&self) -> Option<NodeId> {
        self.hover_node_id
    }

    pub fn get_mousedown_node_id(&self) -> Option<NodeId> {
        self.mousedown_node_id
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        let changes = DeviceChanges::from_viewports(&self.viewport, &viewport);
        self.viewport = viewport;
        self.queue_device_changes(changes);
    }

    /// Returns the current CSS media type used to evaluate `@media` rules.
    pub fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// Sets the CSS media type used to evaluate `@media` rules (e.g. `screen` or `print`)
    /// and queues a stylist device rebuild so updated rules apply on the next restyle.
    pub fn set_media_type(&mut self, media_type: MediaType) {
        if self.media_type == media_type {
            return;
        }
        self.media_type = media_type;
        self.queue_device_changes(DeviceChanges::MEDIA_TYPE);
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn viewport_mut(&mut self) -> ViewportMut<'_> {
        ViewportMut::new(self)
    }

    pub fn zoom_by(&mut self, increment: f32) {
        *self.viewport_mut().zoom_mut() += increment;
    }

    pub fn zoom_to(&mut self, zoom: f32) {
        *self.viewport_mut().zoom_mut() = zoom;
    }

    pub fn get_viewport(&self) -> Viewport {
        self.viewport.clone()
    }

    /// Returns whether incremental layout is currently enabled for this document.
    ///
    /// Both modes run the same style → damage → box construction → layout
    /// pipeline. In incremental mode only nodes carrying `RestyleDamage` (from
    /// mutations or restyles) are reconstructed and have their layout caches
    /// cleared; in non-incremental mode every node is marked damaged at the
    /// start of each `resolve`, so everything is rebuilt each frame.
    pub fn incremental_layout(&self) -> bool {
        self.incremental_layout
    }

    /// Enables or disables incremental layout for this document.
    ///
    /// Disabling it does not switch to a separate code path: all nodes are
    /// simply treated as damaged on every `resolve` (see [`Self::incremental_layout`]).
    pub fn set_incremental_layout(&mut self, enabled: bool) {
        self.incremental_layout = enabled;
    }

    pub fn devtools(&self) -> &DevtoolSettings {
        &self.devtool_settings
    }

    pub fn devtools_mut(&mut self) -> &mut DevtoolSettings {
        &mut self.devtool_settings
    }

    pub fn subdoc(&self, node_id: NodeId) -> Option<&dyn Document> {
        self.get_node(node_id)
            .and_then(|node| node.element_data())
            .and_then(|el| el.sub_doc_data())
    }

    pub fn subdoc_mut(&mut self, node_id: NodeId) -> Option<&mut dyn Document> {
        self.get_node_mut(node_id)
            .and_then(|node| node.element_data_mut())
            .and_then(|el| el.sub_doc_data_mut())
    }

    pub fn is_animating(&self) -> bool {
        #[cfg(feature = "custom-widget")]
        let custom_widget_is_animating = self.custom_widget_nodes.iter().any(|&node_id| {
            self.nodes[node_id]
                .element_data()
                .and_then(|el| el.custom_widget_data())
                .is_some_and(|data| data.widget.requires_redraw())
        });
        #[cfg(not(feature = "custom-widget"))]
        let custom_widget_is_animating = false;

        self.has_canvas
            | self.has_active_animations
            | self.subdoc_is_animating
            | custom_widget_is_animating
            | (self.scroll_animation != ScrollAnimationState::None)
            | self.scrollbars_animating()
    }

    /// Record pending [`DeviceChanges`] and request a redraw so that they are
    /// flushed (via [`Self::flush_pending_device_changes`]) on the next resolve.
    pub(crate) fn queue_device_changes(&mut self, changes: DeviceChanges) {
        if changes.is_empty() {
            return;
        }
        self.pending_device_changes |= changes;
        self.shell_provider.request_redraw();
    }

    /// Apply any pending device changes to the stylist's device, coalescing all
    /// changes since the last flush into a single update of its [`MediaData`](style::device::servo::MediaData).
    pub(crate) fn flush_pending_device_changes(&mut self) {
        let changes = std::mem::take(&mut self.pending_device_changes);
        if changes.is_empty() {
            return;
        }

        let media_data = make_media_data(&self.viewport, self.media_type.clone());
        let device = self.stylist.device_mut();
        let old_viewport_size = device.au_viewport_size();
        device.set_media_data(media_data);
        let viewport_units_changed =
            old_viewport_size != device.au_viewport_size() && device.used_viewport_size();
        self.restyle_for_device_change(viewport_units_changed);
        self.scroll_viewport_by(0.0, 0.0); // Clamp scroll offset

        // Text is shaped at a specific scale factor, so cached inline layouts
        // must be invalidated when the scale changes, as must every node's
        // transform and scrollable overflow (kept in device pixels).
        if changes.contains(DeviceChanges::SCALE) {
            self.invalidate_inline_contexts();
            self.flags.insert(DocumentFlags::RESOLVE_ALL_TRANSFORMS);
        }

        // Color-scheme changes affect values that are resolved at cascade time
        // (`light-dark()`, system colors) without necessarily flipping any
        // media query result, so conservatively recascade the whole tree.
        if changes.contains(DeviceChanges::COLOR_SCHEME) {
            if let Some(root_id) = self.try_root_element().map(|el| el.id) {
                self.nodes[root_id].set_restyle_hint(RestyleHint::recascade_subtree());
            }
        }
    }

    /// Update the device and reset the stylist to process the new size
    pub fn set_stylist_device(&mut self, device: Device) {
        // Seed the new device with the root element's current style and font-relative
        // unit state (used to resolve rem/rlh/rex/rch/rcap/ric units). Stylo only
        // updates this state when the root element's style *changes* during a restyle,
        // so a freshly-built device would otherwise resolve these units against the
        // default font-size (16px) until the root's font-size next changes.
        let root_styles = self
            .try_root_element()
            .and_then(|root| root.primary_styles());
        if let Some(root_style) = root_styles.as_deref() {
            device.set_root_style(root_style);

            let font = root_style.get_font();
            let font_size = font.slow_clone_font_size().computed_size();
            device.set_root_font_size(root_style.effective_zoom.unzoom(font_size.px()));

            let line_height = device
                .calc_line_height(font, root_style.writing_mode, None)
                .0;
            device.set_root_line_height(root_style.effective_zoom.unzoom(line_height.px()));
        }
        drop(root_styles);

        let viewport_size_changed =
            self.stylist.device().au_viewport_size() != device.au_viewport_size();
        let styles_used_viewport_units = self.stylist.device().used_viewport_size();
        *self.stylist.device_mut() = device;
        self.restyle_for_device_change(viewport_size_changed && styles_used_viewport_units);
    }

    /// Invalidate the styles that a change to the stylist's device affects.
    /// `viewport_units_changed` is whether styles that used viewport units need recomputing.
    fn restyle_for_device_change(&mut self, viewport_units_changed: bool) {
        let origins = {
            let guard = &self.guard;
            let guards = StylesheetGuards {
                author: &guard.read(),
                ua_or_user: &guard.read(),
            };
            self.stylist
                .media_features_change_changed_style(&guards, self.stylist.device())
        };

        // Only fully invalidate element styles when the media query results of
        // some origin actually changed. `force_stylesheet_origins_dirty` fully
        // invalidates styles even when `origins` is empty, which would force a
        // full restyle on every viewport resize.
        if !origins.is_empty() {
            self.stylist.force_stylesheet_origins_dirty(origins);
        }

        // Styles that resolve viewport units (vw/vh/etc) still need recomputing
        // when the viewport size changes, even if no media query results
        // changed. Invalidate just the elements whose styles use viewport
        // units (tracked via per-element computed value flags).
        if viewport_units_changed {
            if self
                .stylist
                .get_custom_property_initial_values_flags()
                .intersects(ComputedValueFlags::USES_VIEWPORT_UNITS)
            {
                self.stylist.rebuild_initial_values_for_custom_properties();
            }
            if let Some(root) = self.try_root_element() {
                style::invalidation::viewport_units::invalidate(BlitzNode(root));
            }
        }
    }

    pub fn stylist_device(&mut self) -> &Device {
        self.flush_pending_device_changes();
        self.stylist.device()
    }

    pub fn get_cursor(&self) -> Option<CursorIcon> {
        // Prefer the precise hit node: `cursor` and `user-select` may be set on
        // a pseudo-element or resolved on an anonymous box, and text hits carry
        // is_text via the hit node. Fall back to the canonical hover node if
        // the hit node has been removed (it is transient across resolves), and
        // to the default cursor if the pointer is over no box.
        let Some(node_id) = self
            .hover_hit_node_id
            .filter(|&id| self.nodes.contains_key(id))
            .or(self.get_hover_node_id())
        else {
            return Some(CursorIcon::Default);
        };
        let node = &self.nodes[node_id];

        if let Some(subdoc) = node.subdoc().map(|doc| doc.inner())
            && subdoc.get_hover_node_id().is_some()
        {
            return subdoc.get_cursor();
        }

        let style = node.primary_styles()?;
        let user_select = style.clone_user_select();
        let keyword = style.get_cursor().keyword;

        // Return cursor from style if it is non-auto
        if keyword != CursorKind::Auto {
            return stylo_to_cursor_icon(keyword);
        }

        // Return text cursor for text inputs
        if node
            .element_data()
            .is_some_and(|e| e.text_input_data().is_some())
        {
            return Some(CursorIcon::Text);
        }

        // Use "pointer" cursor if any ancestor is a link. Walk DOM parents: an atomic
        // inline's layout parent is its inline root, which skips the inline elements
        // (such as `<a>`) between them.
        let mut maybe_node = Some(node);
        while let Some(node) = maybe_node {
            if node.data.is_element_with_tag_name(&local_name!("a")) {
                return Some(CursorIcon::Pointer);
            }

            maybe_node = node.parent.map(|node_id| node.with(node_id));
        }

        // Return text cursor for text nodes
        if self.hover_node_is_text {
            return Some(match user_select {
                UserSelect::Text | UserSelect::All | UserSelect::Auto | UserSelect::Contain => {
                    CursorIcon::Text
                }
                UserSelect::None => CursorIcon::Default,
            });
        }

        // Else fallback to default cursor
        Some(CursorIcon::Default)
    }

    pub fn viewport_scroll(&self) -> crate::Point<f64> {
        self.nodes.viewport_scroll()
    }

    pub fn set_viewport_scroll(&mut self, scroll: crate::Point<f64>) {
        self.nodes.set_viewport_scroll(scroll);
    }

    /// Find the node targeted by a URL fragment (the `#...` part of a URL).
    ///
    /// Per the HTML spec, this is the element whose `id` matches the fragment, falling
    /// back to the first `<a>` element whose `name` attribute matches.
    pub fn get_fragment_target(&self, fragment: &str) -> Option<NodeId> {
        if let Some(node_id) = self.get_element_by_id(fragment) {
            return Some(node_id);
        }

        // Fall back to a named anchor: `<a name="...">`
        self.nodes.iter().find_map(|(id, node)| {
            let el = node.element_data()?;
            (el.name.local == local_name!("a") && el.attr(local_name!("name")) == Some(fragment))
                .then_some(id)
        })
    }

    /// `node_id`'s document-relative position and unrounded (sub-pixel) layout in physical
    /// coordinates, for CSSOM geometry APIs such as `getBoundingClientRect`.
    #[cfg(not(feature = "writing-mode"))]
    pub(crate) fn physical_unrounded_geometry(
        &self,
        node_id: NodeId,
    ) -> (crate::util::Point<f32>, taffy::Layout) {
        let mut pos = crate::util::Point { x: 0.0, y: 0.0 };
        let mut current = Some(node_id);
        while let Some(id) = current {
            let node = &self.nodes[id];
            pos.x += node.unrounded_layout().location.x - node.scroll_offset().x as f32;
            pos.y += node.unrounded_layout().location.y - node.scroll_offset().y as f32;
            let cb_id = node.containing_block();
            if node.taffy_position() == taffy::Position::Fixed
                && let Some(cb) = cb_id.and_then(|id| self.nodes.get(id))
                && cb.is_root_element()
            {
                let scroll_offset = *cb.scroll_offset();
                let viewport_scroll = self.viewport_scroll();
                pos.x += scroll_offset.x as f32 + viewport_scroll.x as f32;
                pos.y += scroll_offset.y as f32 + viewport_scroll.y as f32;
            }
            current = cb_id;
        }
        (pos, *self.nodes[node_id].unrounded_layout())
    }

    /// Computes the size and position of the `Node` relative to the viewport
    pub fn get_client_bounding_rect(&self, node_id: NodeId) -> Option<BoundingRect> {
        // Non-atomic inline elements have no layout box of their own: return
        // the union of their per-line-box fragment rects.
        if let Some(rects) = self.inline_fragment_rects(node_id) {
            return rects.reduce(BoundingRect::union);
        }

        self.get_node(node_id)?;
        let (pos, layout) = self.physical_unrounded_geometry(node_id);
        let size = layout.size;

        Some(BoundingRect {
            x: snap_to_layout_unit(pos.x as f64 - self.viewport_scroll().x),
            y: snap_to_layout_unit(pos.y as f64 - self.viewport_scroll().y),
            width: snap_to_layout_unit(size.width as f64),
            height: snap_to_layout_unit(size.height as f64),
        })
    }

    /// CSSOM View's `offsetLeft`/`offsetTop`/`offsetWidth`/`offsetHeight` as a single rect:
    /// the node's border box, positioned relative to the padding edge of its `offsetParent`
    /// (or to the initial containing block when the `offsetParent` is a non-positioned
    /// `body`, or there is none).
    ///
    /// Nodes with their own layout box use that box directly. Non-atomic inline elements
    /// have no layout box: they use the bounding box of their per-line-box fragments, and
    /// their `offsetParent` is resolved from the containing inline root.
    pub fn offset_rect(&self, node_id: NodeId) -> Option<BoundingRect> {
        let node = self.get_node(node_id)?;

        // Nodes with their own layout box: use it directly
        let Some(rects) = self.inline_fragment_rects(node_id) else {
            let pos = node.offset_top_left();
            let size = node.unrounded_layout().size;
            #[cfg(feature = "writing-mode")]
            let size = match node.containing_block() {
                Some(parent) if self.nodes[parent].layout_data().writing_mode.is_vertical() => {
                    size.transpose()
                }
                _ => size,
            };
            return Some(BoundingRect {
                x: pos.x as f64,
                y: pos.y as f64,
                width: size.width as f64,
                height: size.height as f64,
            });
        };

        // Union the per-line-box fragments into a single bounding box
        let bounds = rects.reduce(BoundingRect::union)?;

        // Fragment rects are viewport-relative; convert to document-relative
        let scroll = self.viewport_scroll();
        let mut x = bounds.x + scroll.x;
        let mut y = bounds.y + scroll.y;

        // Resolve the offsetParent from the inline root (the root itself if it is positioned)
        let inline_root = node.inline_root_ancestor()?;
        let offset_parent = if inline_root.is_offset_parent() {
            Some(inline_root)
        } else {
            inline_root.offset_parent()
        };
        // Make the position relative to the offsetParent's padding edge
        if let Some(parent) = offset_parent.filter(|parent| !parent.is_static_body()) {
            let (parent_pos, parent_layout) = self.physical_unrounded_geometry(parent.id);
            let border = parent_layout.border;
            x -= (parent_pos.x + border.left) as f64;
            y -= (parent_pos.y + border.top) as f64;
        }

        Some(BoundingRect {
            x,
            y,
            width: bounds.width,
            height: bounds.height,
        })
    }

    /// Computes the sizes and positions of the `Node`'s box fragments relative to the
    /// viewport (CSSOM `getClientRects()` semantics). Nodes with their own layout box
    /// return a single rect. Non-atomic inline elements (which are laid out as style
    /// spans within an inline root's text layout) return one rect per line box.
    pub fn node_client_rects(&self, node_id: NodeId) -> Vec<BoundingRect> {
        match self.inline_fragment_rects(node_id) {
            Some(rects) => rects.collect(),
            None => self.get_client_bounding_rect(node_id).into_iter().collect(),
        }
    }

    /// Computes per-line-box fragment rects for a non-atomic inline element by walking
    /// the containing inline root's text layout. Returns `None` for nodes that have
    /// their own layout box (which should use `get_client_bounding_rect` instead).
    pub fn inline_fragment_rects(
        &self,
        node_id: NodeId,
    ) -> Option<impl Iterator<Item = BoundingRect> + '_> {
        let node = self.get_node(node_id)?;

        // Only non-atomic inline elements lack their own layout box: they are
        // flattened into the containing inline root's text layout as style spans.
        let fragments = node.inline_fragment_boxes()?;
        let inline_root = node.inline_root_ancestor()?;

        // Fragment boxes are relative to the inline root's border box.
        let (root_pos, _) = self.physical_unrounded_geometry(inline_root.id);
        let origin_x = root_pos.x as f64 - self.viewport_scroll().x;
        let origin_y = root_pos.y as f64 - self.viewport_scroll().y;

        Some(fragments.map(move |rect| BoundingRect {
            x: snap_to_layout_unit(origin_x + rect.left as f64),
            y: snap_to_layout_unit(origin_y + rect.top as f64),
            width: snap_to_layout_unit((rect.right - rect.left) as f64),
            height: snap_to_layout_unit((rect.bottom - rect.top) as f64),
        }))
    }

    /// The first element in tree order with the given tag name. The root element and
    /// its children are checked first as a fast path before a full tree search, making
    /// this suitable for the `documentElement`/`head`/`body` document accessors.
    pub fn find_element_by_tag_name(&self, tag: &LocalName) -> Option<&Node> {
        let root = self.try_root_element()?;
        if root.data.is_element_with_tag_name(tag) {
            return Some(root);
        }
        root.children
            .iter()
            .copied()
            .find(|child_id| {
                self.get_node(*child_id)
                    .is_some_and(|child| child.data.is_element_with_tag_name(tag))
            })
            .or_else(|| {
                TreeTraverser::new(self)
                    .find(|node_id| self.nodes[*node_id].data.is_element_with_tag_name(tag))
            })
            .map(|node_id| &self.nodes[node_id])
    }

    /// The document's `body` element, as for `document.body`
    pub fn find_body_node(&self) -> Option<&Node> {
        self.find_element_by_tag_name(&local_name!("body"))
    }

    /// The document's `head` element, as for `document.head`
    pub fn find_head_node(&self) -> Option<&Node> {
        self.find_element_by_tag_name(&local_name!("head"))
    }

    /// The document's `title` element
    pub fn find_title_node(&self) -> Option<&Node> {
        self.find_element_by_tag_name(&local_name!("title"))
    }

    pub fn with_text_input(
        &mut self,
        node_id: NodeId,
        cb: impl FnOnce(PlainEditorDriver<TextBrush>),
    ) {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return;
        };

        if let Some(text_input) = node
            .element_data_mut()
            .and_then(|el| el.text_input_data_mut())
        {
            let mut font_ctx = self.font_ctx.lock().unwrap();
            let layout_ctx = &mut self.layout_ctx;
            let driver = text_input.editor.driver(&mut font_ctx, layout_ctx);
            cb(driver)
        }
    }

    /// Set the text value of a text input element and refresh its layout.
    pub fn set_text_input_value(&mut self, node_id: NodeId, text: &str) -> bool {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return false;
        };

        if let Some(text_input) = node
            .element_data_mut()
            .and_then(|el| el.text_input_data_mut())
        {
            let mut font_ctx = self.font_ctx.lock().unwrap();
            let layout_ctx = &mut self.layout_ctx;
            text_input.set_text(&mut font_ctx, layout_ctx, text);
            true
        } else {
            false
        }
    }

    /// Recompute the scroll offset of the text input at `node_id` (if any) so that its caret
    /// remains visible within the input's content box.
    pub(crate) fn clamp_text_input_scroll(&mut self, node_id: NodeId) {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return;
        };

        let content_box_width = node.final_layout().content_box_width();
        let content_box_height = node.final_layout().content_box_height();

        if let Some(text_input) = node
            .element_data_mut()
            .and_then(|el| el.text_input_data_mut())
        {
            text_input.clamp_scroll_offset(content_box_width, content_box_height);
        }
    }

    pub(crate) fn compute_has_canvas(&self) -> bool {
        TreeTraverser::new(self).any(|node_id| {
            let node = &self.nodes[node_id];
            let Some(element) = node.element_data() else {
                return false;
            };
            element.name.local == local_name!("canvas") && element.has_attr(local_name!("src"))
        })
    }

    // Text selection methods

    /// Find the text position (inline_root_id, byte_offset) at a given point.
    /// Uses hit() for proper coordinate transformation, then finds the inline root
    /// and byte offset.
    pub fn find_text_position(&self, x: f32, y: f32) -> Option<(NodeId, usize)> {
        let hit = self.hit(x, y)?;
        let hit_node = self.get_node(hit.node_id)?;
        let inline_root = hit_node.inline_root_ancestor()?;
        let byte_offset = inline_root.text_offset_at_point(hit.x, hit.y)?;
        Some((inline_root.id, byte_offset))
    }

    /// Set the text selection range (creates a new selection from anchor to focus)
    pub fn set_text_selection(
        &mut self,
        anchor_node: NodeId,
        anchor_offset: usize,
        focus_node: NodeId,
        focus_offset: usize,
    ) {
        self.text_selection =
            TextSelection::new(anchor_node, anchor_offset, focus_node, focus_offset);

        // For anonymous blocks, switch to storing parent+sibling_index (stable reference)
        if let (Some(parent), Some(idx)) = self.anonymous_block_location(anchor_node) {
            self.text_selection
                .anchor
                .set_anonymous(parent, idx, anchor_offset);
        }
        if let (Some(parent), Some(idx)) = self.anonymous_block_location(focus_node) {
            self.text_selection
                .focus
                .set_anonymous(parent, idx, focus_offset);
        }
    }

    /// Get the parent ID and sibling index for a node if it's an anonymous block.
    /// Returns (None, None) for non-anonymous blocks.
    fn anonymous_block_location(&self, node_id: NodeId) -> (Option<NodeId>, Option<usize>) {
        let Some(node) = self.get_node(node_id) else {
            return (None, None);
        };

        if !node.is_anonymous() {
            return (None, None);
        }

        let Some(parent_id) = node.parent else {
            return (None, None);
        };

        let Some(parent) = self.get_node(parent_id) else {
            return (Some(parent_id), None);
        };

        let layout_children = parent.layout_children.borrow();
        let Some(children) = layout_children.as_ref() else {
            return (Some(parent_id), None);
        };

        // Find the index of this anonymous block among siblings
        let mut anon_index = 0;
        for &child_id in children.iter() {
            if child_id == node_id {
                return (Some(parent_id), Some(anon_index));
            }
            if self.get_node(child_id).is_some_and(|n| n.is_anonymous()) {
                anon_index += 1;
            }
        }

        (Some(parent_id), None)
    }

    /// Clear the text selection
    pub fn clear_text_selection(&mut self) {
        self.text_selection.clear();
    }

    /// Update the selection focus point (used during mouse drag to extend selection).
    pub fn update_selection_focus(&mut self, focus_node: NodeId, focus_offset: usize) {
        // For anonymous blocks, store parent+sibling_index; otherwise store node directly
        if let (Some(parent), Some(idx)) = self.anonymous_block_location(focus_node) {
            self.text_selection
                .focus
                .set_anonymous(parent, idx, focus_offset);
        } else {
            self.text_selection.set_focus(focus_node, focus_offset);
        }
    }

    /// Extend text selection to the given point. Returns true if selection was updated.
    /// This is a convenience method that combines find_text_position and update_selection_focus.
    pub fn extend_text_selection_to_point(&mut self, x: f32, y: f32) -> bool {
        if !self.text_selection.anchor.is_some() {
            return false;
        }

        if let Some((node, offset)) = self.find_text_position(x, y) {
            self.update_selection_focus(node, offset);
            self.shell_provider.request_redraw();
            true
        } else {
            false
        }
    }

    /// Find the Nth anonymous block under a parent.
    fn find_anonymous_block_by_index(
        &self,
        parent_id: NodeId,
        target_index: usize,
    ) -> Option<NodeId> {
        let parent = self.get_node(parent_id)?;
        let layout_children = parent.layout_children.borrow();
        let children = layout_children.as_ref()?;

        children
            .iter()
            .filter(|&&child_id| self.get_node(child_id).is_some_and(|n| n.is_anonymous()))
            .nth(target_index)
            .copied()
    }

    /// Check if there is an active (non-empty) text selection
    pub fn has_text_selection(&self) -> bool {
        self.text_selection.is_active()
    }

    /// Get the selected text content, supporting selection across multiple inline roots.
    pub fn get_selected_text(&self) -> Option<String> {
        let ranges = self.get_text_selection_ranges();
        if ranges.is_empty() {
            return None;
        }

        let mut result = String::new();
        for (node_id, start, end) in &ranges {
            let node = self.get_node(*node_id)?;
            let element_data = node.element_data()?;
            let inline_layout = element_data.inline_layout_data.as_ref()?;

            if *end > inline_layout.text.len() {
                continue;
            }

            if !result.is_empty() {
                result.push(' ');
            }
            let text = &inline_layout.text[*start..*end];
            result.extend(
                text.char_indices()
                    .filter(|&(i, _)| !inline_layout.is_inline_box_char(start + i))
                    .map(|(_, c)| c),
            );
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Get all selection ranges as Vec<(node_id, start_offset, end_offset)>.
    /// Returns empty vec if no selection.
    pub fn get_text_selection_ranges(&self) -> Vec<(NodeId, usize, usize)> {
        let lookup = |parent_id, idx| self.find_anonymous_block_by_index(parent_id, idx);

        let anchor_node = match self.text_selection.anchor.resolve_node_id(lookup) {
            Some(id) => id,
            None => return Vec::new(),
        };
        let focus_node = match self.text_selection.focus.resolve_node_id(lookup) {
            Some(id) => id,
            None => return Vec::new(),
        };

        // Guard against stale selection endpoints: nodes may have been removed from
        // the document (e.g. by script) since the selection was made.
        let node_is_in_doc = |node_id: NodeId| {
            self.nodes
                .get(node_id)
                .is_some_and(|node| node.flags.is_in_document())
        };
        if !node_is_in_doc(anchor_node) || !node_is_in_doc(focus_node) {
            return Vec::new();
        }

        // Single node selection
        if anchor_node == focus_node {
            let start = self
                .text_selection
                .anchor
                .offset
                .min(self.text_selection.focus.offset);
            let end = self
                .text_selection
                .anchor
                .offset
                .max(self.text_selection.focus.offset);

            if start == end {
                return Vec::new();
            }
            return vec![(anchor_node, start, end)];
        }

        // Multi-node selection: collect all inline roots between anchor and focus
        let inline_roots = self.collect_inline_roots_in_range(anchor_node, focus_node);
        if inline_roots.is_empty() {
            return Vec::new();
        }

        // Determine document order using the collected inline_roots order
        // (inline_roots is already in document order from first to last)
        let first_in_roots = inline_roots[0];

        let (first_node, first_offset, last_node, last_offset) =
            if first_in_roots == anchor_node || (first_in_roots != focus_node) {
                // anchor is first (or neither endpoint is in roots, which shouldn't happen)
                (
                    anchor_node,
                    self.text_selection.anchor.offset,
                    focus_node,
                    self.text_selection.focus.offset,
                )
            } else {
                // focus is first
                (
                    focus_node,
                    self.text_selection.focus.offset,
                    anchor_node,
                    self.text_selection.anchor.offset,
                )
            };

        let mut ranges = Vec::with_capacity(inline_roots.len());

        for &node_id in &inline_roots {
            let Some(node) = self.get_node(node_id) else {
                continue;
            };
            let Some(element_data) = node.element_data() else {
                continue;
            };
            let Some(inline_layout) = element_data.inline_layout_data.as_ref() else {
                continue;
            };

            let text_len = inline_layout.text.len();

            if node_id == first_node && node_id == last_node {
                let start = first_offset.min(last_offset);
                let end = first_offset.max(last_offset);
                if start < end && end <= text_len {
                    ranges.push((node_id, start, end));
                }
            } else if node_id == first_node {
                if first_offset < text_len {
                    ranges.push((node_id, first_offset, text_len));
                }
            } else if node_id == last_node {
                if last_offset > 0 && last_offset <= text_len {
                    ranges.push((node_id, 0, last_offset));
                }
            } else if text_len > 0 {
                ranges.push((node_id, 0, text_len));
            }
        }

        ranges
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BoundingRect {
    /// The smallest rect containing both `self` and `other`.
    pub fn union(self, other: BoundingRect) -> BoundingRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        BoundingRect {
            x,
            y,
            width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y,
        }
    }
}

/// Snap a CSSOM geometry value to a 1/64px grid (the precision of Blink's `LayoutUnit`).
/// Layout values are accumulated in `f32`, so e.g. seven `55/7`-wide flex items would
/// otherwise end at `55.0000005` and appear to overflow their 55px container.
fn snap_to_layout_unit(value: f64) -> f64 {
    (value * 64.0).round() / 64.0
}

impl Drop for BaseDocument {
    fn drop(&mut self) {
        #[cfg(feature = "devtools")]
        if let Some(provider) = &self.devtools_provider {
            provider.document_dropped();
        }
    }
}

impl AsRef<BaseDocument> for BaseDocument {
    fn as_ref(&self) -> &BaseDocument {
        self
    }
}

impl AsMut<BaseDocument> for BaseDocument {
    fn as_mut(&mut self) -> &mut BaseDocument {
        self
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;
    use blitz_traits::shell::ColorScheme;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingShellProvider {
        redraw_count: AtomicUsize,
    }
    impl ShellProvider for CountingShellProvider {
        fn request_redraw(&self) {
            self.redraw_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn zoom_requests_redraw() {
        let shell_provider = Arc::new(CountingShellProvider::default());
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            shell_provider: Some(shell_provider.clone() as _),
            ..Default::default()
        });
        doc.resolve(0.0);

        let base = shell_provider.redraw_count.load(Ordering::SeqCst);
        doc.zoom_by(0.5);
        assert!(shell_provider.redraw_count.load(Ordering::SeqCst) > base);

        let base = shell_provider.redraw_count.load(Ordering::SeqCst);
        doc.zoom_to(1.0);
        assert!(shell_provider.redraw_count.load(Ordering::SeqCst) > base);
    }
}

#[cfg(test)]
mod hover_state_tests {
    use super::*;
    use crate::{Attribute, qual_name};
    use blitz_traits::shell::ColorScheme;

    /// Build `<html><body style="margin:0"><div style="width:300px">some text
    /// <div style="height:50px"></div></div></body></html>` manually (the HTML
    /// parser lives in blitz-html, which would be a circular dev-dependency).
    /// The bare text next to a block sibling gets wrapped in an anonymous
    /// block, which becomes the inline root: text hits report the anonymous
    /// block as the hit node.
    fn make_doc() -> (BaseDocument, NodeId) {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };

        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(qual_name!("body"), vec![style("margin:0")]);
        let container = mutator.create_element(qual_name!("div"), vec![style("width:300px")]);
        let text = mutator.create_text_node("some text");
        let block = mutator.create_element(qual_name!("div"), vec![style("height:50px")]);
        mutator.append_children(container, &[text, block]);
        mutator.append_children(body, &[container]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        doc.resolve(0.0);
        (doc, container)
    }

    /// Whether text laid out with a real (non-zero-metric) font. Without the
    /// `system-fonts` feature text measures 0x0 and text hits are impossible,
    /// making these tests vacuous.
    fn text_has_size(doc: &BaseDocument, container: NodeId) -> bool {
        doc.nodes[container].final_layout().size.height > 50.0
    }

    /// Regression test: hovering bare text wrapped in an anonymous block must
    /// report a text cursor. The hit node for such text is the anonymous
    /// inline root itself, while the *stored* hover target is canonicalized to
    /// the containing element — the cursor must be derived from the precise
    /// hit node, not the canonical target.
    #[test]
    fn hovering_text_in_anonymous_block_reports_text_cursor() {
        let (mut doc, container) = make_doc();
        if !text_has_size(&doc, container) {
            eprintln!("skipping: no usable font (text measures 0x0)");
            return;
        }

        doc.set_hover_to(5.0, 8.0);
        assert!(doc.hover_node_is_text, "expected a text hit");
        let hit_id = doc.hover_hit_node_id.expect("expected a hit node");
        assert!(
            doc.nodes[hit_id].is_anonymous(),
            "expected the hit node to be the anonymous inline root"
        );
        assert_eq!(
            doc.get_hover_node_id(),
            Some(container),
            "expected the stored hover target to be the containing element"
        );
        assert_eq!(doc.get_cursor(), Some(CursorIcon::Text));
    }

    /// Hovering the empty region of the anonymous block (right of the text) is
    /// not a text hit: default cursor, same canonical hover target.
    #[test]
    fn hovering_anonymous_block_whitespace_reports_default_cursor() {
        let (mut doc, container) = make_doc();
        if !text_has_size(&doc, container) {
            eprintln!("skipping: no usable font (text measures 0x0)");
            return;
        }

        doc.set_hover_to(250.0, 8.0);
        assert!(!doc.hover_node_is_text);
        assert_eq!(doc.get_hover_node_id(), Some(container));
        assert_eq!(doc.get_cursor(), Some(CursorIcon::Default));
    }

    /// Hovering below the content hits no box: default cursor (`None` would hide it).
    #[test]
    fn hovering_below_content_reports_default_cursor() {
        let (mut doc, _) = make_doc();

        doc.set_hover_to(5.0, 250.0);
        assert_eq!(doc.get_hover_node_id(), None);
        assert_eq!(doc.get_cursor(), Some(CursorIcon::Default));
    }
}

#[cfg(test)]
mod hover_invalidation_tests {
    use super::*;
    use crate::{Attribute, QualName, qual_name};
    use blitz_traits::shell::ColorScheme;

    /// Build `<html><body style="margin:0"><div style="width:300px;height:100px">
    /// <span>text</span></div></body></html>` with a `div:hover` rule.
    fn make_doc() -> (BaseDocument, NodeId, NodeId) {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let sheet = UserAgentStylesheet::parse(
            "div:hover { background-color: rgb(255, 0, 0); } div:hover span { color: rgb(0, 255, 0); }",
        );
        doc.add_user_agent_stylesheet(sheet);
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };

        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(qual_name!("body"), vec![style("margin:0")]);
        let div =
            mutator.create_element(qual_name!("div"), vec![style("width:300px;height:100px")]);
        let span = mutator.create_element(qual_name!("span"), vec![]);
        let text = mutator.create_text_node("some text");
        mutator.append_children(span, &[text]);
        mutator.append_children(div, &[span]);
        mutator.append_children(body, &[div]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        doc.resolve(0.0);
        (doc, div, span)
    }

    fn bg_color(doc: &BaseDocument, id: NodeId) -> String {
        format!(
            "{:?}",
            doc.nodes[id]
                .primary_styles()
                .unwrap()
                .get_background()
                .background_color
        )
    }

    fn text_color(doc: &BaseDocument, id: NodeId) -> String {
        format!(
            "{:?}",
            doc.nodes[id].primary_styles().unwrap().clone_color()
        )
    }

    #[test]
    fn hover_styles_apply_and_clear() {
        let (mut doc, div, span) = make_doc();
        let initial_bg = bg_color(&doc, div);
        let initial_color = text_color(&doc, span);

        // Hover the div
        doc.set_hover_to(10.0, 10.0);
        assert!(doc.nodes[div].is_hovered());
        doc.resolve(0.0);
        let hovered_bg = bg_color(&doc, div);
        let hovered_color = text_color(&doc, span);
        assert_ne!(initial_bg, hovered_bg, "hover should change div background");
        assert_ne!(
            initial_color, hovered_color,
            "hover should change span color"
        );

        // Move the pointer off the div (below it, over the body)
        doc.set_hover_to(10.0, 200.0);
        assert!(!doc.nodes[div].is_hovered());
        doc.resolve(0.0);
        assert_eq!(
            bg_color(&doc, div),
            initial_bg,
            "unhover should restore div background"
        );
        assert_eq!(
            text_color(&doc, span),
            initial_color,
            "unhover should restore span color"
        );
    }

    /// Mimics BBC's headline hover pattern: `a:link:hover .headline` with the
    /// headline several levels below the anchor.
    #[test]
    fn ancestor_hover_with_link_state_updates_descendant() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let sheet = UserAgentStylesheet::parse(
            ".promo:link:hover .headline, .promo:visited:hover .headline { color: rgb(184, 0, 0); text-decoration-line: underline; }",
        );
        doc.add_user_agent_stylesheet(sheet);
        let root_id = doc.root_node().id;
        let attr = |name: QualName, value: &str| Attribute {
            name,
            value: value.to_string(),
        };

        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(
            qual_name!("body"),
            vec![attr(qual_name!("style"), "margin:0")],
        );
        let a = mutator.create_element(
            qual_name!("a"),
            vec![
                attr(qual_name!("href"), "https://example.com"),
                attr(qual_name!("class"), "promo"),
                attr(
                    qual_name!("style"),
                    "display:block;width:300px;height:100px",
                ),
            ],
        );
        let p = mutator.create_element(qual_name!("p"), vec![]);
        let span = mutator.create_element(
            qual_name!("span"),
            vec![attr(qual_name!("class"), "headline")],
        );
        let text = mutator.create_text_node("Headline text");
        mutator.append_children(span, &[text]);
        mutator.append_children(p, &[span]);
        mutator.append_children(a, &[p]);
        mutator.append_children(body, &[a]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        doc.resolve(0.0);
        let initial_color = text_color(&doc, span);

        doc.set_hover_to(10.0, 10.0);
        assert!(doc.nodes[a].is_hovered());
        doc.resolve(0.0);
        let hovered_color = text_color(&doc, span);
        assert_ne!(
            initial_color, hovered_color,
            "hovering the anchor should change the headline color"
        );

        doc.set_hover_to(10.0, 200.0);
        assert!(!doc.nodes[a].is_hovered());
        doc.resolve(0.0);
        assert_eq!(
            text_color(&doc, span),
            initial_color,
            "unhovering the anchor should restore the headline color"
        );
    }

    /// Toggling a checkbox must invalidate `:checked`-dependent styles.
    #[test]
    fn checkbox_toggle_updates_checked_styles() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let sheet = UserAgentStylesheet::parse(
            "input:checked { color: rgb(184, 0, 0); } input:checked + label { color: rgb(0, 184, 0); }",
        );
        doc.add_user_agent_stylesheet(sheet);
        let root_id = doc.root_node().id;
        let attr = |name: QualName, value: &str| Attribute {
            name,
            value: value.to_string(),
        };

        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(
            qual_name!("body"),
            vec![attr(qual_name!("style"), "margin:0")],
        );
        let input = mutator.create_element(
            qual_name!("input"),
            vec![attr(qual_name!("type"), "checkbox")],
        );
        let label = mutator.create_element(qual_name!("label"), vec![]);
        let text = mutator.create_text_node("label text");
        mutator.append_children(label, &[text]);
        mutator.append_children(body, &[input, label]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        doc.resolve(0.0);
        let initial_input_color = text_color(&doc, input);
        let initial_label_color = text_color(&doc, label);

        // Toggle the checkbox on (as the click handler does)
        doc.snapshot_node_and(input, ElementState::CHECKED, |node| {
            if let Some(el) = node.element_data_mut() {
                BaseDocument::toggle_checkbox(el);
            }
            node.mark_ancestors_dirty();
        });
        doc.resolve(0.0);
        assert_ne!(
            text_color(&doc, input),
            initial_input_color,
            "checking should change the input color"
        );
        assert_ne!(
            text_color(&doc, label),
            initial_label_color,
            "checking should change the sibling label color"
        );

        // Toggle the checkbox back off
        doc.snapshot_node_and(input, ElementState::CHECKED, |node| {
            if let Some(el) = node.element_data_mut() {
                BaseDocument::toggle_checkbox(el);
            }
            node.mark_ancestors_dirty();
        });
        doc.resolve(0.0);
        assert_eq!(
            text_color(&doc, input),
            initial_input_color,
            "unchecking should restore the input color"
        );
        assert_eq!(
            text_color(&doc, label),
            initial_label_color,
            "unchecking should restore the sibling label color"
        );
    }

    /// A repaint-only style change to an existing `::before` pseudo-element
    /// (no box construction damage) must be flushed to the pseudo-element's
    /// anonymous node during the style traversal.
    #[test]
    fn hover_updates_existing_pseudo_element_style() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let sheet = UserAgentStylesheet::parse(
            "div::before { content: \"x\"; color: rgb(1, 2, 3); } \
             div:hover::before { color: rgb(0, 0, 255); }",
        );
        doc.add_user_agent_stylesheet(sheet);
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };

        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(qual_name!("body"), vec![style("margin:0")]);
        let div = mutator.create_element(
            qual_name!("div"),
            vec![style("display:block;width:300px;height:100px")],
        );
        let text = mutator.create_text_node("some text");
        mutator.append_children(div, &[text]);
        mutator.append_children(body, &[div]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        doc.resolve(0.0);
        let before = doc.nodes[div].before().expect("::before node should exist");
        let initial_color = text_color(&doc, before);

        doc.set_hover_to(10.0, 10.0);
        assert!(doc.nodes[div].is_hovered());
        doc.resolve(0.0);
        assert_ne!(
            text_color(&doc, before),
            initial_color,
            "hover should change the ::before color"
        );

        doc.set_hover_to(10.0, 200.0);
        assert!(!doc.nodes[div].is_hovered());
        doc.resolve(0.0);
        assert_eq!(
            text_color(&doc, before),
            initial_color,
            "unhover should restore the ::before color"
        );
    }

    /// Background-image layers must be flushed to the node's dedicated image
    /// storage when its style changes (queued during the style traversal),
    /// now that image flushing no longer runs on every node in the flush pass.
    #[test]
    fn hover_updates_background_image_layers() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let sheet = UserAgentStylesheet::parse(
            "div { background-image: url(\"https://example.com/a.png\"); } \
             div:hover { background-image: url(\"https://example.com/b.png\"); }",
        );
        doc.add_user_agent_stylesheet(sheet);
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };

        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(qual_name!("body"), vec![style("margin:0")]);
        let div = mutator.create_element(
            qual_name!("div"),
            vec![style("display:block;width:300px;height:100px")],
        );
        mutator.append_children(body, &[div]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        let background_image_url = |doc: &BaseDocument, id: NodeId| -> Option<String> {
            let elem = doc.nodes[id].data.downcast_element().unwrap();
            elem.background_images
                .first()
                .and_then(|img| img.as_ref())
                .map(|img| img.url.as_str().to_string())
        };

        doc.resolve(0.0);
        assert_eq!(
            background_image_url(&doc, div).as_deref(),
            Some("https://example.com/a.png"),
            "initial resolve should flush the background image"
        );

        doc.set_hover_to(10.0, 10.0);
        assert!(doc.nodes[div].is_hovered());
        doc.resolve(0.0);
        assert_eq!(
            background_image_url(&doc, div).as_deref(),
            Some("https://example.com/b.png"),
            "hover should flush the changed background image"
        );

        doc.set_hover_to(10.0, 200.0);
        assert!(!doc.nodes[div].is_hovered());
        doc.resolve(0.0);
        assert_eq!(
            background_image_url(&doc, div).as_deref(),
            Some("https://example.com/a.png"),
            "unhover should flush the restored background image"
        );
    }
}

#[cfg(test)]
mod picture_display_tests {
    use super::*;
    use crate::qual_name;
    use blitz_traits::shell::ColorScheme;

    /// `<picture>` must generate a box (inline, as in browsers): the
    /// aspect-ratio wrapper pattern relies on it for its absolutely positioned
    /// child. `display: contents` would leave the `<img>` in flow instead.
    #[test]
    fn picture_is_not_display_contents() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(qual_name!("body"), vec![]);
        let picture = mutator.create_element(qual_name!("picture"), vec![]);
        mutator.append_children(body, &[picture]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        doc.resolve(0.0);
        let display = doc.nodes[picture]
            .primary_styles()
            .expect("picture should be styled")
            .get_box()
            .display;
        assert!(!display.is_contents());
    }
}

#[cfg(test)]
mod inline_with_only_block_children_tests {
    use super::*;
    use crate::qual_name;
    use blitz_traits::shell::ColorScheme;

    #[test]
    fn inline_with_only_block_children_is_detected() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        doc.add_user_agent_stylesheet(crate::UserAgentStylesheet::parse(
            "html, body, div { display: block } b { display: contents }",
        ));
        let root_id = doc.root_node().id;
        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        mutator.append_children(root_id, &[html]);
        let body = mutator.create_element(qual_name!("body"), vec![]);
        mutator.append_children(html, &[body]);

        let inline_with_block = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[inline_with_block]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(inline_with_block, &[block_child]);

        let inline_with_whitespace_and_block = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[inline_with_whitespace_and_block]);
        let whitespace = mutator.create_text_node("\n  ");
        mutator.append_children(inline_with_whitespace_and_block, &[whitespace]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(inline_with_whitespace_and_block, &[block_child]);

        let inline_with_text_and_block = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[inline_with_text_and_block]);
        let text = mutator.create_text_node("text");
        mutator.append_children(inline_with_text_and_block, &[text]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(inline_with_text_and_block, &[block_child]);

        // The nested span is itself laid out as a block box, so the outer span has
        // block-level content only. Chrome paints nothing for the outer span either.
        let inline_with_nested_block = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[inline_with_nested_block]);
        let nested_inline = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(inline_with_nested_block, &[nested_inline]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(nested_inline, &[block_child]);

        let inline_with_comment_and_block = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[inline_with_comment_and_block]);
        let comment = mutator.create_comment_node("comment");
        mutator.append_children(inline_with_comment_and_block, &[comment]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(inline_with_comment_and_block, &[block_child]);

        let inline_with_contents_text_and_block =
            mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[inline_with_contents_text_and_block]);
        let contents = mutator.create_element(qual_name!("b"), vec![]);
        mutator.append_children(inline_with_contents_text_and_block, &[contents]);
        let text = mutator.create_text_node("text");
        mutator.append_children(contents, &[text]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(inline_with_contents_text_and_block, &[block_child]);

        let inline_with_text = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[inline_with_text]);
        let text = mutator.create_text_node("text");
        mutator.append_children(inline_with_text, &[text]);

        let empty_inline = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[empty_inline]);

        let block_with_block = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(body, &[block_with_block]);
        let nested_block = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(block_with_block, &[nested_block]);
        drop(mutator);

        doc.resolve(0.0);
        assert!(doc.nodes[inline_with_block].is_inline_with_only_block_children());
        assert!(doc.nodes[inline_with_whitespace_and_block].is_inline_with_only_block_children());
        assert!(!doc.nodes[inline_with_text_and_block].is_inline_with_only_block_children());
        assert!(doc.nodes[inline_with_nested_block].is_inline_with_only_block_children());
        assert!(doc.nodes[inline_with_comment_and_block].is_inline_with_only_block_children());
        assert!(
            !doc.nodes[inline_with_contents_text_and_block].is_inline_with_only_block_children()
        );
        assert!(!doc.nodes[inline_with_text].is_inline_with_only_block_children());
        assert!(!doc.nodes[empty_inline].is_inline_with_only_block_children());
        assert!(!doc.nodes[block_with_block].is_inline_with_only_block_children());
    }

    #[test]
    fn inline_with_only_block_children_follows_child_edits() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        doc.add_user_agent_stylesheet(crate::UserAgentStylesheet::parse(
            "html, body, div { display: block }",
        ));
        let root_id = doc.root_node().id;
        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        mutator.append_children(root_id, &[html]);
        let body = mutator.create_element(qual_name!("body"), vec![]);
        mutator.append_children(html, &[body]);
        let span = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[span]);
        let text = mutator.create_text_node("text");
        mutator.append_children(span, &[text]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(span, &[block_child]);
        drop(mutator);
        doc.resolve(0.0);
        assert!(!doc.nodes[span].is_inline_with_only_block_children());

        let mut mutator = doc.mutate();
        mutator.remove_and_drop_node(text);
        drop(mutator);
        doc.resolve(0.0);
        assert!(doc.nodes[span].is_inline_with_only_block_children());

        let mut mutator = doc.mutate();
        let inline_child = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(span, &[inline_child]);
        drop(mutator);
        doc.resolve(0.0);
        assert!(!doc.nodes[span].is_inline_with_only_block_children());
    }

    #[test]
    fn inline_with_only_block_children_follows_text_edits() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        doc.add_user_agent_stylesheet(crate::UserAgentStylesheet::parse(
            "html, body, div { display: block }",
        ));
        let root_id = doc.root_node().id;
        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        mutator.append_children(root_id, &[html]);
        let body = mutator.create_element(qual_name!("body"), vec![]);
        mutator.append_children(html, &[body]);
        let span = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[span]);
        let whitespace = mutator.create_text_node("\n  ");
        mutator.append_children(span, &[whitespace]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(span, &[block_child]);
        drop(mutator);
        doc.resolve(0.0);
        assert!(doc.nodes[span].is_inline_with_only_block_children());

        let mut mutator = doc.mutate();
        mutator.append_text_to_node(whitespace, "x").unwrap();
        drop(mutator);
        doc.resolve(0.0);
        assert!(!doc.nodes[span].is_inline_with_only_block_children());
    }

    #[test]
    fn inline_with_only_block_children_follows_contents_descendant_edits() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        doc.add_user_agent_stylesheet(crate::UserAgentStylesheet::parse(
            "html, body, div { display: block } b { display: contents }",
        ));
        let root_id = doc.root_node().id;
        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        mutator.append_children(root_id, &[html]);
        let body = mutator.create_element(qual_name!("body"), vec![]);
        mutator.append_children(html, &[body]);
        let span = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[span]);
        let contents = mutator.create_element(qual_name!("b"), vec![]);
        mutator.append_children(span, &[contents]);
        let block_in_contents = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(contents, &[block_in_contents]);
        let block_child = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(span, &[block_child]);
        drop(mutator);
        doc.resolve(0.0);
        assert!(doc.nodes[span].is_inline_with_only_block_children());

        let mut mutator = doc.mutate();
        let text = mutator.create_text_node("text");
        mutator.append_children(contents, &[text]);
        drop(mutator);
        doc.resolve(0.0);
        assert!(!doc.nodes[span].is_inline_with_only_block_children());
    }
}

#[cfg(test)]
mod font_face_override_tests {
    use super::*;
    use crate::net::{FontFaceOverrides, Resource, ResourceLoadResponse};

    /// Regression-pin for the `@font-face` descriptor-honouring fix.
    ///
    /// The bug was that `Resource::Font` carried only the raw font bytes,
    /// so `load_resource` registered fonts with `info_override = None` and
    /// parley fell back to the TTF's internal `name` table. After the fix,
    /// `Resource::Font` carries `FontFaceOverrides` and `load_resource`
    /// builds a `FontInfoOverride` from them — meaning a CSS-declared
    /// `font-family` alias wins over the file's own metadata.
    ///
    /// We drive `load_resource` directly with a fabricated response rather
    /// than go through HTML parsing → `fetch_font_face`, because the
    /// downstream HTML parser lives in `blitz-html` (would be a circular
    /// crate dependency). The mapping from `@font-face` descriptors into
    /// `FontFaceOverrides` is covered by the unit tests in `net.rs`; this
    /// test pins the load-side of the pipeline.
    #[test]
    fn font_face_overrides_alias_family_name() {
        const ALIAS: &str = "AliasedFamily";

        let mut document = BaseDocument::new(DocumentConfig::default());

        // Sanity: the alias name is not registered before we feed the font.
        {
            let mut ctx = document.font_ctx.lock().unwrap();
            assert!(
                ctx.collection.family_id(ALIAS).is_none(),
                "alias must not exist before registration",
            );
        }

        // Drive `load_resource` with a `Resource::Font` whose overrides
        // assert the CSS-side family name. We use the bullet font as a
        // valid font payload — its internal `name` table is irrelevant to
        // the assertion; what matters is whether the override wins.
        let response = ResourceLoadResponse {
            request_id: 0,
            node_id: None,
            resolved_url: Some(String::from("test://aliased-family")),
            result: Ok(Resource::Font(
                blitz_traits::net::Bytes::from_static(crate::BULLET_FONT),
                FontFaceOverrides {
                    family_name: Some(String::from(ALIAS)),
                    weight: Some(800.0),
                    style: Some(parley::fontique::FontStyle::Italic),
                },
            )),
        };
        document.load_resource(response);

        // The override must have taken effect: parley's `Collection` now
        // resolves the CSS-declared alias to a registered family.
        let mut ctx = document.font_ctx.lock().unwrap();
        let family_id = ctx
            .collection
            .family_id(ALIAS)
            .expect("CSS-declared family name should be registered as a family alias");
        let resolved_name = ctx
            .collection
            .family_name(family_id)
            .expect("family id should resolve back to a name");
        assert_eq!(
            resolved_name, ALIAS,
            "registered family should report the CSS-declared name, \
             not the font file's internal `name` table entry",
        );
    }
}

#[cfg(test)]
mod change_tracking_tests {
    use super::*;
    use crate::qual_name;
    use blitz_traits::shell::ColorScheme;

    #[test]
    fn mutations_and_focus_changes_are_tracked() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(qual_name!("body"), vec![]);
        let button = mutator.create_element(qual_name!("button"), vec![]);
        mutator.append_children(body, &[button]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);
        doc.resolve(0.0);
        assert!(doc.has_changes());

        doc.clear_changes();
        assert!(!doc.has_changes());

        doc.mutate()
            .set_attribute(button, qual_name!("aria-label"), "Submit");
        assert!(doc.has_changes());

        doc.clear_changes();
        assert!(doc.set_focus_to(button));
        assert!(doc.has_changes());

        doc.clear_changes();
        doc.clear_focus();
        assert!(doc.has_changes());
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use crate::{Attribute, qual_name};
    use blitz_traits::SmolStr;
    use blitz_traits::events::{
        BlitzAnimationEvent, BlitzPointerEvent, BlitzPointerId, BlitzTransitionEvent, DomEventData,
        EventState, PointerCoords,
    };
    use blitz_traits::shell::ColorScheme;

    #[test]
    fn test_flush_animation_events_lock_free_drain() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let target = doc.root_node().id;
        let anim_data = BlitzAnimationEvent {
            animation_name: "fade-in".to_string(),
            elapsed_time: 1.0,
            pseudo_element: SmolStr::default(),
        };
        doc.pending_animation_events.push(QueuedAnimationEvent {
            node_id: target,
            event: DomEventData::AnimationStart(anim_data),
        });

        assert_eq!(doc.pending_animation_events.len(), 1);

        let mut received = Vec::new();
        doc.flush_animation_events(|evt| {
            received.push(evt);
        });

        assert_eq!(doc.pending_animation_events.len(), 0);
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].target, target);
        assert!(received[0].bubbles);
        assert!(!received[0].cancelable);
        assert_eq!(received[0].data.name(), "animationstart");
    }

    #[derive(Default, Clone)]
    struct EventCollector {
        events: Rc<RefCell<Vec<DomEvent>>>,
    }

    impl crate::events::EventHandler for EventCollector {
        fn handle_event(
            &mut self,
            _chain: &[NodeId],
            event: &mut DomEvent,
            _doc: &mut dyn Document,
            _event_state: &mut EventState,
        ) {
            self.events.borrow_mut().push(event.clone());
        }
    }

    #[test]
    fn test_event_driver_animation_dispatch() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let target = doc.root_node().id;
        doc.pending_animation_events.push(QueuedAnimationEvent {
            node_id: target,
            event: DomEventData::AnimationStart(BlitzAnimationEvent {
                animation_name: "slide".to_string(),
                elapsed_time: 0.0,
                pseudo_element: SmolStr::default(),
            }),
        });
        doc.pending_animation_events.push(QueuedAnimationEvent {
            node_id: target,
            event: DomEventData::TransitionEnd(BlitzTransitionEvent {
                property_name: "opacity".to_string(),
                elapsed_time: 0.3,
                pseudo_element: SmolStr::default(),
            }),
        });

        let collector = EventCollector::default();
        let mut driver = EventDriver::new(&mut doc, collector.clone());
        driver.flush_animation_events();

        let events = collector.events.borrow();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data.name(), "animationstart");
        assert_eq!(events[1].data.name(), "transitionend");
        assert_eq!(events[0].target, target);
        assert_eq!(events[1].target, target);
        assert!(events[0].bubbles);
        assert!(!events[0].cancelable);
        assert!(events[1].bubbles);
        assert!(!events[1].cancelable);
    }

    #[test]
    fn test_missing_node_panic_guard() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let non_existent_node_id = NodeId::from_u64(99999);
        doc.pending_animation_events.push(QueuedAnimationEvent {
            node_id: non_existent_node_id,
            event: DomEventData::AnimationStart(BlitzAnimationEvent {
                animation_name: "ghost".to_string(),
                elapsed_time: 0.0,
                pseudo_element: SmolStr::default(),
            }),
        });

        // Test BaseDocument::flush_animation_events
        let mut received = Vec::new();
        doc.flush_animation_events(|evt| received.push(evt));
        assert!(received.is_empty());
        assert!(doc.pending_animation_events.is_empty());

        // Test EventDriver::flush_animation_events
        doc.pending_animation_events.push(QueuedAnimationEvent {
            node_id: non_existent_node_id,
            event: DomEventData::AnimationStart(BlitzAnimationEvent {
                animation_name: "ghost".to_string(),
                elapsed_time: 0.0,
                pseudo_element: SmolStr::default(),
            }),
        });
        let collector = EventCollector::default();
        let mut driver = EventDriver::new(&mut doc, collector.clone());
        driver.flush_animation_events();
        assert!(collector.events.borrow().is_empty());
    }

    #[test]
    fn test_trait_flush_animation_events_drains_buffer() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let target = doc.root_node().id;
        doc.pending_animation_events.push(QueuedAnimationEvent {
            node_id: target,
            event: DomEventData::AnimationStart(BlitzAnimationEvent {
                animation_name: "test-anim".to_string(),
                elapsed_time: 0.0,
                pseudo_element: SmolStr::default(),
            }),
        });
        assert_eq!(doc.pending_animation_events.len(), 1);

        Document::flush_animation_events(&mut doc);

        assert_eq!(doc.pending_animation_events.len(), 0);
    }

    #[test]
    fn test_real_css_transition_lifecycle() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };
        let div_id = {
            let mut mutator = doc.mutate();
            let div = mutator.create_element(
                qual_name!("div"),
                vec![style("transition: opacity 0.2s linear; opacity: 1;")],
            );
            mutator.append_children(root_id, &[div]);
            div
        };
        // Initial resolve
        doc.resolve(0.0);
        let mut events = Vec::new();
        doc.flush_animation_events(|e| events.push(e));
        assert!(events.is_empty());

        // Change opacity to trigger transition
        {
            let mut mutator = doc.mutate();
            mutator.set_style_property(div_id, "opacity", "0");
        }
        doc.resolve(0.0);
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data.name(), "transitionrun");
        assert_eq!(events[1].data.name(), "transitionstart");

        // Advance to 0.2s: transition finishes
        events.clear();
        doc.resolve(0.2);
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.name(), "transitionend");
        if let DomEventData::TransitionEnd(ref end) = events[0].data {
            assert_eq!(end.property_name, "opacity");
            assert_eq!(end.elapsed_time, 0.2);
        } else {
            panic!("Expected TransitionEnd");
        }
    }

    #[test]
    fn test_real_css_transition_negative_delay() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };
        let div_id = {
            let mut mutator = doc.mutate();
            let div = mutator.create_element(
                qual_name!("div"),
                vec![style("transition: opacity 0.2s linear -0.05s; opacity: 1;")],
            );
            mutator.append_children(root_id, &[div]);
            div
        };
        doc.resolve(0.0);

        {
            let mut mutator = doc.mutate();
            mutator.set_style_property(div_id, "opacity", "0");
        }
        doc.resolve(0.0);
        let mut events = Vec::new();
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data.name(), "transitionrun");
        assert_eq!(events[1].data.name(), "transitionstart");
        if let DomEventData::TransitionStart(ref start) = events[1].data {
            assert!((start.elapsed_time - 0.05).abs() < 1e-4);
        } else {
            panic!("Expected TransitionStart");
        }
    }

    #[test]
    fn test_real_css_transition_cancellation_on_detach() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };
        let div_id = {
            let mut mutator = doc.mutate();
            let div = mutator.create_element(
                qual_name!("div"),
                vec![style("transition: opacity 1.0s linear; opacity: 1;")],
            );
            mutator.append_children(root_id, &[div]);
            div
        };
        doc.resolve(0.0);

        {
            let mut mutator = doc.mutate();
            mutator.set_style_property(div_id, "opacity", "0");
        }
        doc.resolve(0.0);
        let mut events = Vec::new();
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 2); // run and start

        // Detach div from tree
        {
            let mut mutator = doc.mutate();
            mutator.remove_node(div_id);
        }
        events.clear();
        doc.resolve(0.5);
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.name(), "transitioncancel");
        if let DomEventData::TransitionCancel(ref cancel) = events[0].data {
            assert_eq!(cancel.property_name, "opacity");
            assert!((cancel.elapsed_time - 0.5).abs() < 1e-4);
        } else {
            panic!("Expected TransitionCancel");
        }
    }

    #[test]
    fn test_real_css_animation_zero_duration_sequencing() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        doc.add_user_agent_stylesheet(UserAgentStylesheet::parse(
            "@keyframes instant-fade { from { opacity: 0; } to { opacity: 1; } }",
        ));
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };
        {
            let mut mutator = doc.mutate();
            let div = mutator.create_element(
                qual_name!("div"),
                vec![style("animation: instant-fade 0s;")],
            );
            mutator.append_children(root_id, &[div]);
        }

        doc.resolve(0.0);
        let mut events = Vec::new();
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data.name(), "animationstart");
        assert_eq!(events[1].data.name(), "animationend");
        if let DomEventData::AnimationStart(ref start) = events[0].data {
            assert_eq!(start.animation_name, "instant-fade");
            assert_eq!(start.elapsed_time, 0.0);
        }
        if let DomEventData::AnimationEnd(ref end) = events[1].data {
            assert_eq!(end.animation_name, "instant-fade");
            assert_eq!(end.elapsed_time, 0.0);
        }
    }

    #[test]
    fn test_real_css_animation_multi_iteration_coalescing() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        doc.add_user_agent_stylesheet(UserAgentStylesheet::parse(
            "@keyframes pulse { from { opacity: 0; } to { opacity: 1; } }",
        ));
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };
        {
            let mut mutator = doc.mutate();
            let div = mutator.create_element(
                qual_name!("div"),
                vec![style("animation: pulse 0.1s linear 2;")],
            );
            mutator.append_children(root_id, &[div]);
        }

        // Creation at t=0.0
        doc.resolve(0.0);
        let mut events = Vec::new();
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.name(), "animationstart");

        // Advance to t=0.1 (iteration 1 completes, iteration 2 begins)
        events.clear();
        doc.resolve(0.1);
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.name(), "animationiteration");
        if let DomEventData::AnimationIteration(ref iter) = events[0].data {
            assert!((iter.elapsed_time - 0.1).abs() < 1e-4);
        }

        // Advance to t=0.2 (iteration 2 completes and animation finishes)
        // Per W3C spec, animationend fires and animationiteration MUST BE SUPPRESSED!
        events.clear();
        doc.resolve(0.2);
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.name(), "animationend");
        if let DomEventData::AnimationEnd(ref end) = events[0].data {
            assert!((end.elapsed_time - 0.2).abs() < 1e-4);
        }
    }

    #[test]
    fn test_real_css_animation_cancellation_on_detach() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        doc.add_user_agent_stylesheet(UserAgentStylesheet::parse(
            "@keyframes inf-spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }",
        ));
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };
        let div_id = {
            let mut mutator = doc.mutate();
            let div = mutator.create_element(
                qual_name!("div"),
                vec![style("animation: inf-spin 1.0s linear infinite;")],
            );
            mutator.append_children(root_id, &[div]);
            div
        };

        doc.resolve(0.0);
        let mut events = Vec::new();
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.name(), "animationstart");

        // Detach div
        {
            let mut mutator = doc.mutate();
            mutator.remove_node(div_id);
        }
        events.clear();
        doc.resolve(0.5);
        doc.flush_animation_events(|e| events.push(e));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.name(), "animationcancel");
        if let DomEventData::AnimationCancel(ref cancel) = events[0].data {
            assert_eq!(cancel.animation_name, "inf-spin");
            assert!((cancel.elapsed_time - 0.5).abs() < 1e-4);
        }
    }

    #[test]
    fn test_event_capture_hook() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root_id = doc.root_node().id;
        let (child_id, grandchild_id) = {
            let mut mutator = doc.mutate();
            let child = mutator.create_element(qual_name!("div"), vec![]);
            mutator.append_children(root_id, &[child]);
            let grandchild = mutator.create_element(qual_name!("button"), vec![]);
            mutator.append_children(child, &[grandchild]);
            (child, grandchild)
        };

        let captured = Rc::new(RefCell::new(Vec::new()));
        let captured_clone = captured.clone();
        doc.set_capture_hook(Some(Rc::new(move |_doc, event, chain| {
            captured_clone
                .borrow_mut()
                .push((event.clone(), chain.to_vec()));
        })));

        assert!(doc.capture_hook().is_some());

        let collector = EventCollector::default();
        let mut driver = EventDriver::new(&mut doc, collector.clone());

        // 1. Verify AnimationStart event dispatch triggers hook
        let anim_data = BlitzAnimationEvent {
            animation_name: "test".to_string(),
            elapsed_time: 0.0,
            pseudo_element: SmolStr::default(),
        };
        let event = DomEvent::new(child_id, DomEventData::AnimationStart(anim_data));
        driver.handle_dom_event(event);

        {
            let cap = captured.borrow();
            assert_eq!(
                cap.len(),
                1,
                "Capture hook should be called exactly once for animation"
            );
            assert_eq!(cap[0].0.target, child_id);
            assert_eq!(cap[0].1, vec![child_id]);
        }

        // 2. Verify PointerDown event triggers hook and receives full node chain
        let ptr_data = BlitzPointerEvent {
            id: BlitzPointerId::Mouse,
            is_primary: true,
            coords: PointerCoords {
                page_x: 10.0,
                page_y: 10.0,
                screen_x: 10.0,
                screen_y: 10.0,
                client_x: 10.0,
                client_y: 10.0,
            },
            mods: Default::default(),
            button: Default::default(),
            buttons: Default::default(),
            details: Default::default(),
            element: Default::default(),
            active_pointers: Default::default(),
        };
        let ptr_event = DomEvent::new(grandchild_id, DomEventData::PointerDown(ptr_data));
        driver.handle_dom_event(ptr_event);

        {
            let cap = captured.borrow();
            assert_eq!(
                cap.len(),
                2,
                "Capture hook should be called for PointerDown"
            );
            assert_eq!(cap[1].0.target, grandchild_id);
            assert_eq!(cap[1].1, vec![grandchild_id, child_id]);
        }

        let handled = collector.events.borrow();
        assert_eq!(handled.len(), 2, "Handler should receive both events");
        assert_eq!(handled[0].target, child_id);
        assert_eq!(handled[1].target, grandchild_id);

        // Test clearing hook
        drop(driver);
        doc.set_capture_hook(None);
        assert!(doc.capture_hook().is_none());
    }
}

#[cfg(test)]
mod geometry_viewport_scroll_tests {
    use super::*;
    use crate::{Attribute, Point, qual_name};
    use blitz_traits::shell::{ColorScheme, Viewport};

    fn make_test_doc() -> (BaseDocument, NodeId, NodeId) {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let style = |value: &str| Attribute {
            name: qual_name!("style"),
            value: value.to_string(),
        };

        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(
            qual_name!("body"),
            vec![style("margin: 0; width: 800px; height: 2000px;")],
        );
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);
        (doc, html, body)
    }

    fn create_styled_div(doc: &mut BaseDocument, style: &str) -> NodeId {
        let full_style = if style.contains("display:") {
            style.to_string()
        } else {
            format!("display: block; {style}")
        };
        let mut mutator = doc.mutate();
        mutator.create_element(
            qual_name!("div"),
            vec![Attribute {
                name: qual_name!("style"),
                value: full_style,
            }],
        )
    }

    #[test]
    fn test_fixed_element_bounding_rect_invariant_under_viewport_scroll() {
        let (mut doc, _html, body) = make_test_doc();
        let fixed_id = create_styled_div(
            &mut doc,
            "position: fixed; top: 20px; left: 30px; width: 100px; height: 50px;",
        );
        let normal_id = create_styled_div(
            &mut doc,
            "position: static; margin-top: 40px; margin-left: 10px; width: 200px; height: 80px;",
        );
        let absolute_id = create_styled_div(
            &mut doc,
            "position: absolute; top: 150px; left: 60px; width: 120px; height: 60px;",
        );

        {
            let mut mutator = doc.mutate();
            mutator.append_children(body, &[fixed_id, normal_id, absolute_id]);
        }

        doc.resolve(0.0);

        // 1. Initial state at viewport_scroll = (0.0, 0.0)
        assert_eq!(doc.viewport_scroll(), Point { x: 0.0, y: 0.0 });
        let fixed_rect_initial = doc.get_client_bounding_rect(fixed_id).unwrap();
        let normal_rect_initial = doc.get_client_bounding_rect(normal_id).unwrap();
        let absolute_rect_initial = doc.get_client_bounding_rect(absolute_id).unwrap();

        assert_eq!(fixed_rect_initial.x, 30.0);
        assert_eq!(fixed_rect_initial.y, 20.0);
        assert_eq!(fixed_rect_initial.width, 100.0);
        assert_eq!(fixed_rect_initial.height, 50.0);

        // Verify initial absolute_position
        let fixed_node = doc.get_node(fixed_id).unwrap();
        let fixed_abs_initial = fixed_node.absolute_position(0.0, 0.0);
        assert_eq!(fixed_abs_initial.x, 30.0);
        assert_eq!(fixed_abs_initial.y, 20.0);

        // 2a. Isolated X-only viewport scroll
        doc.set_viewport_scroll(Point { x: 75.0, y: 0.0 });
        assert_eq!(
            doc.get_client_bounding_rect(fixed_id).unwrap(),
            fixed_rect_initial,
            "Fixed element get_client_bounding_rect invariant under X-only scroll"
        );

        // 2b. Isolated Y-only viewport scroll
        doc.set_viewport_scroll(Point { x: 0.0, y: 150.0 });
        assert_eq!(
            doc.get_client_bounding_rect(fixed_id).unwrap(),
            fixed_rect_initial,
            "Fixed element get_client_bounding_rect invariant under Y-only scroll"
        );

        // 2c. Multi-axis X+Y viewport scroll to (50.0, 200.0)
        doc.set_viewport_scroll(Point { x: 50.0, y: 200.0 });
        assert_eq!(doc.viewport_scroll(), Point { x: 50.0, y: 200.0 });

        let fixed_rect_scrolled = doc.get_client_bounding_rect(fixed_id).unwrap();
        let normal_rect_scrolled = doc.get_client_bounding_rect(normal_id).unwrap();
        let absolute_rect_scrolled = doc.get_client_bounding_rect(absolute_id).unwrap();

        // Fixed element client rect must be EXACTLY identical after scrolling
        assert_eq!(
            fixed_rect_scrolled, fixed_rect_initial,
            "Fixed element get_client_bounding_rect must be strictly invariant under viewport scroll"
        );

        // Normal and absolute element client rects must shift by (-50.0, -200.0)
        assert_eq!(
            normal_rect_scrolled.x,
            normal_rect_initial.x - 50.0,
            "Normal element x should shift by scroll x"
        );
        assert_eq!(
            normal_rect_scrolled.y,
            normal_rect_initial.y - 200.0,
            "Normal element y should shift by scroll y"
        );
        assert_eq!(normal_rect_scrolled.width, normal_rect_initial.width);
        assert_eq!(normal_rect_scrolled.height, normal_rect_initial.height);

        assert_eq!(
            absolute_rect_scrolled.x,
            absolute_rect_initial.x - 50.0,
            "Absolute element x should shift by scroll x"
        );
        assert_eq!(
            absolute_rect_scrolled.y,
            absolute_rect_initial.y - 200.0,
            "Absolute element y should shift by scroll y"
        );
        assert_eq!(absolute_rect_scrolled.width, absolute_rect_initial.width);
        assert_eq!(absolute_rect_scrolled.height, absolute_rect_initial.height);

        // Fixed element absolute_position (Document-relative) must shift by (+50.0, +200.0)
        let fixed_node = doc.get_node(fixed_id).unwrap();
        let fixed_abs_scrolled = fixed_node.absolute_position(0.0, 0.0);
        assert_eq!(
            fixed_abs_scrolled.x,
            fixed_abs_initial.x + 50.0,
            "Fixed element document absolute_position x should include viewport scroll"
        );
        assert_eq!(
            fixed_abs_scrolled.y,
            fixed_abs_initial.y + 200.0,
            "Fixed element document absolute_position y should include viewport scroll"
        );

        // offset_rect must also remain invariant under viewport scroll
        let fixed_offset = doc.offset_rect(fixed_id).unwrap();
        assert_eq!(fixed_offset.x, 30.0);
        assert_eq!(fixed_offset.y, 20.0);
    }

    #[test]
    fn test_fixed_element_inside_transformed_container_scrolls_with_container() {
        let (mut doc, _html, body) = make_test_doc();
        // A container with transform creates a containing block for fixed descendants per CSS Transforms Module §2.
        let transformed_container = create_styled_div(
            &mut doc,
            "transform: scale(1); margin-top: 100px; margin-left: 40px; width: 400px; height: 300px;",
        );
        let fixed_child = create_styled_div(
            &mut doc,
            "position: fixed; top: 10px; left: 15px; width: 80px; height: 40px;",
        );

        {
            let mut mutator = doc.mutate();
            mutator.append_children(transformed_container, &[fixed_child]);
            mutator.append_children(body, &[transformed_container]);
        }

        doc.resolve(0.0);

        let initial_rect = doc.get_client_bounding_rect(fixed_child).unwrap();

        // Scroll viewport
        doc.set_viewport_scroll(Point { x: 30.0, y: 70.0 });

        let scrolled_rect = doc.get_client_bounding_rect(fixed_child).unwrap();

        // Because containing block is the transformed container (not the root element),
        // the fixed element must scroll with the document/container!
        assert_eq!(
            scrolled_rect.x,
            initial_rect.x - 30.0,
            "Fixed element inside transformed container must shift with viewport scroll x"
        );
        assert_eq!(
            scrolled_rect.y,
            initial_rect.y - 70.0,
            "Fixed element inside transformed container must shift with viewport scroll y"
        );
    }
}

#[cfg(test)]
mod pointer_capture_tests {
    use super::*;
    use crate::qual_name;
    use blitz_traits::events::{MouseEventButton, Point, PointerCoords, PointerDetails};
    use blitz_traits::shell::ColorScheme;

    fn sample_pointer_event(id: BlitzPointerId, buttons: MouseEventButtons) -> BlitzPointerEvent {
        BlitzPointerEvent {
            id,
            is_primary: true,
            coords: PointerCoords {
                page_x: 10.0,
                page_y: 10.0,
                screen_x: 10.0,
                screen_y: 10.0,
                client_x: 10.0,
                client_y: 10.0,
            },
            button: MouseEventButton::Main,
            buttons,
            mods: Default::default(),
            details: PointerDetails::default(),
            element: Point::default(),
            active_pointers: Default::default(),
        }
    }

    #[test]
    fn pointer_capture_state_validation_and_slab_reuse() {
        let mut doc = BaseDocument::new(DocumentConfig {
            viewport: Some(Viewport::new(200, 200, 1.0, ColorScheme::Light)),
            ..Default::default()
        });
        let root_id = doc.root_node().id;
        let mut mutator = doc.mutate();
        let html = mutator.create_element(qual_name!("html"), vec![]);
        let body = mutator.create_element(qual_name!("body"), vec![]);
        let a = mutator.create_element(qual_name!("div"), vec![]);
        let b = mutator.create_element(qual_name!("div"), vec![]);
        let disconnected = mutator.create_element(qual_name!("div"), vec![]);
        mutator.append_children(body, &[a, b]);
        mutator.append_children(html, &[body]);
        mutator.append_children(root_id, &[html]);
        drop(mutator);

        assert_eq!(
            doc.set_pointer_capture(a, 1),
            Err(PointerCaptureError::NotFound)
        );
        assert_eq!(
            doc.release_pointer_capture(a, 1),
            Err(PointerCaptureError::NotFound)
        );
        assert!(!doc.has_pointer_capture(a, 1));

        doc.active_pointers.insert(
            1,
            ActivePointerRecord {
                id: BlitzPointerId::Mouse,
                is_primary: true,
                buttons: MouseEventButtons::None,
                last_event: sample_pointer_event(BlitzPointerId::Mouse, MouseEventButtons::None),
                element_under_pointer: Some(a),
            },
        );
        assert_eq!(
            doc.set_pointer_capture(disconnected, 1),
            Err(PointerCaptureError::InvalidState)
        );
        assert_eq!(doc.set_pointer_capture(a, 1), Ok(()));
        assert!(!doc.has_pointer_capture(a, 1));

        doc.active_pointers.get_mut(&1).unwrap().buttons = MouseEventButtons::Primary;
        assert_eq!(doc.set_pointer_capture(a, 1), Ok(()));
        assert!(doc.has_pointer_capture(a, 1));
        assert!(!doc.has_pointer_capture(b, 1));

        assert_eq!(doc.release_pointer_capture(b, 1), Ok(()));
        assert!(doc.has_pointer_capture(a, 1));

        assert_eq!(doc.set_pointer_capture(b, 1), Ok(()));
        assert!(!doc.has_pointer_capture(a, 1));
        assert!(doc.has_pointer_capture(b, 1));

        doc.pointer_capture_targets.insert(
            1,
            CaptureTarget::Connected {
                target: b,
                explicit: true,
            },
        );
        let mut mutator = doc.mutate();
        mutator.remove_and_drop_node(b);
        let reused = mutator.create_element(qual_name!("span"), vec![]);
        mutator.append_children(body, &[reused]);
        drop(mutator);

        assert_eq!(
            doc.pointer_capture_targets.get(&1),
            Some(&CaptureTarget::DisconnectedPendingLostCapture)
        );
        assert!(!doc.has_pointer_capture(reused, 1));
        assert_eq!(doc.active_capture_node(BlitzPointerId::Mouse), None);
    }
}
