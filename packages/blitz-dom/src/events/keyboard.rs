use crate::events::focus::generate_focus_events;
use crate::traversal::TreeTraverser;
use crate::{
    BaseDocument,
    node::GeneratedTextInputEvent,
    util::{ACTION_MOD, action_key},
};
use blitz_traits::node_id::NodeId;
use blitz_traits::{
    SmolStr,
    events::{BlitzInputEvent, BlitzKeyEvent, DomEvent, DomEventData},
};
use keyboard_types::{Key, Modifiers, NamedKey};

pub(super) enum KeyboardOrTextInputEvent {
    KeyPress(BlitzKeyEvent),
    AppleStandardKeyBinding(SmolStr),
}

pub(crate) fn handle_key_or_input_event<F: FnMut(DomEvent)>(
    doc: &mut BaseDocument,
    target: NodeId,
    event: KeyboardOrTextInputEvent,
    mut dispatch_event: F,
) {
    if let KeyboardOrTextInputEvent::KeyPress(event) = &event {
        if event.is_composing {
            return;
        }

        if event.key == Key::Named(NamedKey::Tab)
            && !event
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::META)
        {
            let backwards = event.modifiers.contains(Modifiers::SHIFT);
            generate_focus_events(
                doc,
                &mut |doc| {
                    if backwards {
                        doc.focus_prev_node();
                    } else {
                        doc.focus_next_node();
                    }
                },
                &mut dispatch_event,
            );
            return;
        }

        if event.key == Key::Named(NamedKey::Escape) {
            doc.clear_keyboard_active_node();
            doc.clear_focus();
            return;
        }

        // Handle copy (Ctrl+C/Cmd+C) for text selection when no text input is focused
        if event.state.is_pressed() {
            let action_mod = event.modifiers.contains(ACTION_MOD);
            if action_mod && action_key(event) == Some('c') {
                // Check if we have a text selection (and no focused text input)
                let has_focused_text_input = doc.focus_node_id.is_some_and(|id| {
                    doc.get_node(id)
                        .and_then(|n| n.element_data())
                        .is_some_and(|e| e.text_input_data().is_some())
                });

                if !has_focused_text_input {
                    if let Some(text) = doc.get_selected_text() {
                        let _ = doc.shell_provider.set_clipboard_text(text);
                        return;
                    }
                }
            }
        }
    }

    if let Some(node_id) = doc.focus_node_id {
        if target != node_id {
            return;
        }

        if !doc.is_node_displayed(node_id) || doc.is_form_control_disabled(node_id) {
            return;
        }

        let has_text_input = doc.nodes[node_id]
            .element_data()
            .is_some_and(|el| el.text_input_data().is_some());

        if has_text_input {
            let generated_event = {
                let node = &mut doc.nodes[node_id];
                let input_data = node
                    .element_data_mut()
                    .and_then(|el| el.text_input_data_mut())
                    .unwrap();
                match event {
                    KeyboardOrTextInputEvent::KeyPress(blitz_key_event) => input_data
                        .apply_keypress_event(
                            &mut doc.font_ctx.lock().unwrap(),
                            &mut doc.layout_ctx,
                            &*doc.shell_provider,
                            blitz_key_event,
                        ),
                    KeyboardOrTextInputEvent::AppleStandardKeyBinding(command) => input_data
                        .apply_apple_standard_keybinding(
                            &mut doc.font_ctx.lock().unwrap(),
                            &mut doc.layout_ctx,
                            &*doc.shell_provider,
                            &command,
                        ),
                }
            };

            if let Some(generated_event) = generated_event {
                doc.apply_generated_text_input_event(node_id, generated_event, dispatch_event);
            }
            return;
        }

        if let KeyboardOrTextInputEvent::KeyPress(key_event) = event {
            if !key_event.state.is_pressed() {
                return;
            }

            match &key_event.key {
                Key::Named(NamedKey::Enter) => {
                    if key_event.is_auto_repeating {
                        return;
                    }
                    let is_link = doc.nodes[node_id]
                        .element_data()
                        .is_some_and(|el| el.is_link());
                    if is_link {
                        if !key_event.modifiers.contains(Modifiers::ALT) {
                            doc.click_count = 0;
                            doc.last_mousedown_time = None;
                            let click =
                                doc.nodes[node_id].synthetic_click_event(key_event.modifiers);
                            dispatch_event(DomEvent::new(node_id, click));
                        }
                    } else if doc.is_enter_non_link_activatable_element(node_id)
                        && !key_event
                            .modifiers
                            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::META)
                    {
                        doc.click_count = 0;
                        doc.last_mousedown_time = None;
                        let click = doc.nodes[node_id].synthetic_click_event(key_event.modifiers);
                        dispatch_event(DomEvent::new(node_id, click));
                    }
                }
                Key::Character(s) if s == " " => {
                    if !key_event.modifiers.intersects(
                        Modifiers::CONTROL | Modifiers::ALT | Modifiers::META | Modifiers::SHIFT,
                    ) && doc.is_space_activatable_element(node_id)
                    {
                        if !key_event.is_auto_repeating {
                            doc.set_keyboard_active_node(node_id);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

pub(crate) fn handle_keyup_event<F: FnMut(DomEvent)>(
    doc: &mut BaseDocument,
    target: NodeId,
    event: &BlitzKeyEvent,
    mut dispatch_event: F,
) {
    if event.is_composing || !matches!(&event.key, Key::Character(s) if s == " ") {
        return;
    }

    let armed = doc.take_keyboard_active_node();
    if armed == Some(target)
        && doc.focus_node_id == Some(target)
        && doc.is_node_displayed(target)
        && !doc.is_form_control_disabled(target)
        && doc.is_space_activatable_element(target)
    {
        doc.click_count = 0;
        doc.last_mousedown_time = None;
        let click = doc.nodes[target].synthetic_click_event(event.modifiers);
        dispatch_event(DomEvent::new(target, click));
    }
}

impl BaseDocument {
    pub(crate) fn apply_generated_text_input_event<F: FnMut(DomEvent)>(
        &mut self,
        node_id: NodeId,
        event: GeneratedTextInputEvent,
        mut dispatch_event: F,
    ) {
        let node = &mut self.nodes[node_id];
        let element_data = node
            .element_data_mut()
            .expect("apply_generated_text_input_event called on a node that is not an element");
        let input_data = element_data
            .text_input_data_mut()
            .expect("apply_generated_text_input_event called on a node that is not a text input");

        match event {
            GeneratedTextInputEvent::Input => {
                let value = input_data.editor.raw_text().to_string();
                dispatch_event(DomEvent::new(
                    node_id,
                    DomEventData::Input(BlitzInputEvent { value }),
                ));
                self.shell_provider.request_redraw();
            }
            GeneratedTextInputEvent::Select | GeneratedTextInputEvent::PreEditChange => {
                self.shell_provider.request_redraw();
            }
            GeneratedTextInputEvent::Submit => {
                implicit_form_submission(self, node_id, dispatch_event);
            }
        }
    }
}

/// https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#implicit-submission
fn implicit_form_submission<F: FnMut(DomEvent)>(
    doc: &mut BaseDocument,
    text_target: NodeId,
    mut dispatch_event: F,
) {
    let Some(&form_owner_id) = doc.controls_to_form.get(&text_target) else {
        return;
    };

    // HTML §4.10.21.2: If the form has a default button (first submit button in tree order
    // belonging to the form), and that button is not disabled, fire a synthetic click at it.
    let default_button_id = TreeTraverser::new(doc).find(|&control_id| {
        doc.controls_to_form.get(&control_id) == Some(&form_owner_id)
            && doc
                .get_node(control_id)
                .and_then(|n| n.element_data())
                .is_some_and(|el| el.is_submit_button() || el.is_submit_input())
    });

    if let Some(btn_id) = default_button_id {
        if !doc.is_form_control_disabled(btn_id) && doc.is_node_displayed(btn_id) {
            doc.click_count = 0;
            doc.last_mousedown_time = None;
            let click = doc.nodes[btn_id].synthetic_click_event(Modifiers::empty());
            dispatch_event(DomEvent::new(btn_id, click));
        }
        return;
    }

    let blocking_fields = doc
        .controls_to_form
        .iter()
        .filter(|(_, form_id)| **form_id == form_owner_id)
        .filter_map(|(control_id, _)| doc.get_node(*control_id)?.element_data())
        .filter(|element_data| element_data.blocks_implicit_submission())
        .count();

    if blocking_fields > 1 {
        return;
    }

    doc.submit_form(form_owner_id, form_owner_id);
}
