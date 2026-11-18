use blitz_dom::{Document, EventDriver, EventHandler};
use blitz_test_harness::{Harness, mouse_pointer_event};
use blitz_traits::events::{DomEvent, DomEventData, EventState, UiEvent};
use blitz_traits::node_id::NodeId;
use std::cell::RefCell;
use std::rc::Rc;

fn is_checked(harness: &Harness, selector: &str) -> bool {
    let id = harness.node(selector);
    harness
        .base()
        .get_node(id)
        .and_then(|n| n.element_data())
        .and_then(|el| el.checkbox_input_checked())
        .unwrap()
}

fn click_recording_targets(
    harness: &mut Harness,
    selector: &str,
    cancel_click_target: Option<NodeId>,
) -> Vec<NodeId> {
    #[derive(Clone)]
    struct ClickRecorder {
        clicks: Rc<RefCell<Vec<NodeId>>>,
        cancel_click_target: Option<NodeId>,
    }

    impl EventHandler for ClickRecorder {
        fn handle_event(
            &mut self,
            _chain: &[NodeId],
            event: &mut DomEvent,
            _doc: &mut dyn Document,
            event_state: &mut EventState,
        ) {
            if matches!(event.data, DomEventData::Click(_)) {
                self.clicks.borrow_mut().push(event.target);
                if self.cancel_click_target == Some(event.target) {
                    event_state.prevent_default();
                }
            }
        }
    }

    let clicks = Rc::new(RefCell::new(Vec::new()));
    let (x, y) = harness.center_of(selector);
    let ptr = mouse_pointer_event(x, y);
    {
        let mut doc = harness.base_mut();
        let mut driver = EventDriver::new(
            &mut *doc,
            ClickRecorder {
                clicks: Rc::clone(&clicks),
                cancel_click_target,
            },
        );
        driver.handle_ui_event(UiEvent::PointerDown(ptr.clone()));
        driver.handle_ui_event(UiEvent::PointerUp(ptr));
    }
    harness.pump();
    Rc::try_unwrap(clicks).unwrap().into_inner()
}

#[test]
fn implicit_label_binds_first_labelable_and_guards_interactive_descendants() {
    let mut harness = Harness::from_html(
        r#"<html><body style="margin:0">
            <label id="implicit" style="display:block;">
                <span id="caption" style="display:block; height:20px;">Label</span>
                <button id="btn" type="button" style="display:block; width:80px; height:20px;">B</button>
                <input id="cb" type="checkbox" style="display:block; width:20px; height:20px;">
            </label>
            <label for="explicit_cb" style="display:block;">
                <button id="nested_btn" type="button" style="display:block; width:80px; height:20px;">Nested</button>
            </label>
            <input id="explicit_cb" type="checkbox" style="display:block; width:20px; height:20px;">
        </body></html>"#,
    );

    let bound = harness
        .base()
        .label_bound_input_element(harness.node("#implicit"))
        .map(|n| n.id);
    assert_eq!(bound, Some(harness.node("#btn")));

    harness.click("#caption");
    assert_eq!(harness.focused(), Some(harness.node("#btn")));
    assert!(!is_checked(&harness, "#cb"));

    harness.click("#btn");
    assert!(!is_checked(&harness, "#cb"));

    harness.click("#nested_btn");
    assert!(!is_checked(&harness, "#explicit_cb"));
}

#[test]
fn implicit_label_skips_hidden_input() {
    let mut harness = Harness::from_html(
        r#"<html><body style="margin:0">
            <label id="lbl" style="display:block; height:20px;">
                Agree
                <input type="HIDDEN" value="token">
                <input id="cb" type="checkbox">
            </label>
        </body></html>"#,
    );

    harness.click("#lbl");
    assert!(is_checked(&harness, "#cb"));
    assert_eq!(harness.focused(), Some(harness.node("#cb")));
}

#[test]
fn explicit_label_resolves_select_textarea_and_button() {
    let mut harness = Harness::from_html(
        r#"<html><body style="margin:0">
            <label id="l_sel" for="sel" style="display:block; height:20px;">Select</label>
            <select id="sel"><option>A</option></select>
            <label id="l_txt" for="txt" style="display:block; height:20px;">Notes</label>
            <textarea id="txt"></textarea>
            <label id="l_btn" for="btn" style="display:block; height:20px;">Action</label>
            <button id="btn" type="button">Go</button>
        </body></html>"#,
    );

    for (label, control) in [("#l_sel", "#sel"), ("#l_txt", "#txt"), ("#l_btn", "#btn")] {
        harness.click(label);
        assert_eq!(harness.focused(), Some(harness.node(control)));
    }
}

#[test]
fn label_wrapping_range_input_terminates() {
    let mut harness = Harness::from_html(
        r#"<html><body style="margin:0">
            <label id="hue" style="display:block; width:180px; height:40px;">
                <span id="caption" style="display:block; height:20px;">Hue</span>
                <input id="range" type="range" min="0" max="100" style="display:block; width:120px; height:20px;">
            </label>
        </body></html>"#,
    );

    harness.click("#caption");
    assert_eq!(harness.focused(), Some(harness.node("#range")));
    harness.click("#range");
}

#[test]
fn clicking_label_focuses_text_controls_unless_disabled() {
    let mut harness = Harness::from_html(
        r#"<html><body style="margin:0">
            <label id="l_input" for="inp" style="display:block; height:20px;">Name</label>
            <input id="inp" type="text">
            <label id="l_area" style="display:block;">
                <span id="area_caption" style="display:block; height:20px;">Bio</span>
                <textarea id="area"></textarea>
            </label>
            <label id="l_dis" for="dis" style="display:block; height:20px;">Disabled</label>
            <input id="dis" type="text" disabled>
        </body></html>"#,
    );

    harness.click("#l_input");
    assert_eq!(harness.focused(), Some(harness.node("#inp")));

    harness.click("#area_caption");
    assert_eq!(harness.focused(), Some(harness.node("#area")));

    harness.click("#l_dis");
    assert_ne!(harness.focused(), Some(harness.node("#dis")));
}

#[test]
fn clicking_label_dispatches_synthetic_click_to_enabled_button_and_checkbox_once() {
    let mut harness = Harness::from_html(
        r#"<html><body style="margin:0">
            <label id="l_for_btn" for="btn_explicit" style="display:block; width:100px; height:20px;">Explicit</label>
            <button id="btn_explicit" type="button" style="display:block; width:80px; height:20px;">Go</button>

            <label id="l_wrap_btn" style="display:block; width:120px;">
                <span id="wrap_caption" style="display:block; height:20px;">Text</span>
                <button id="btn_wrapped" type="button" style="display:block; width:80px; height:20px;">Wrapped</button>
            </label>

            <label id="l_for_dis" for="btn_disabled" style="display:block; width:100px; height:20px;">Disabled Explicit</label>
            <button id="btn_disabled" type="button" disabled style="display:block; width:80px; height:20px;">Disabled</button>

            <fieldset disabled style="margin:0; padding:0; border:0;">
                <label id="l_fieldset_dis" style="display:block; width:120px;">
                    <span id="fieldset_caption" style="display:block; height:20px;">Fieldset Disabled</span>
                    <button id="btn_fieldset_dis" type="button" style="display:block; width:80px; height:20px;">Blocked</button>
                </label>
            </fieldset>

            <label id="l_for_cb" for="cb_single" style="display:block; width:100px; height:20px;">Toggle</label>
            <input id="cb_single" type="checkbox" style="display:block; width:20px; height:20px;">
        </body></html>"#,
    );

    let l_for_btn = harness.node("#l_for_btn");
    let btn_explicit = harness.node("#btn_explicit");
    let wrap_caption = harness.node("#wrap_caption");
    let btn_wrapped = harness.node("#btn_wrapped");
    let l_for_dis = harness.node("#l_for_dis");
    let btn_disabled = harness.node("#btn_disabled");
    let fieldset_caption = harness.node("#fieldset_caption");
    let btn_fieldset_dis = harness.node("#btn_fieldset_dis");
    let l_for_cb = harness.node("#l_for_cb");
    let cb_single = harness.node("#cb_single");

    // 1. Explicit <label for="btn_explicit"> dispatches Click on label then synthetic Click on button,
    //    focusing #btn_explicit with Pointer modality (:focus == true, :focus-visible == false).
    let explicit_clicks = click_recording_targets(&mut harness, "#l_for_btn", None);
    assert_eq!(explicit_clicks, vec![l_for_btn, btn_explicit]);
    assert_eq!(harness.focused(), Some(btn_explicit));
    assert!(harness.query("#btn_explicit:focus").is_some());
    assert!(harness.query("#btn_explicit:focus-visible").is_none());

    // 2. Implicit <label><span>Text</span><button id="btn_wrapped">...</button></label>
    //    dispatches synthetic Click on #btn_wrapped when clicking caption, and only 1 Click when clicking button directly.
    let wrapped_caption_clicks = click_recording_targets(&mut harness, "#wrap_caption", None);
    assert_eq!(wrapped_caption_clicks, vec![wrap_caption, btn_wrapped]);
    assert_eq!(harness.focused(), Some(btn_wrapped));
    assert!(harness.query("#btn_wrapped:focus").is_some());
    assert!(harness.query("#btn_wrapped:focus-visible").is_none());

    let direct_btn_clicks = click_recording_targets(&mut harness, "#btn_wrapped", None);
    assert_eq!(direct_btn_clicks, vec![btn_wrapped]);

    // 3. Disabled button (both direct `disabled` and `<fieldset disabled>`) receives zero synthetic Clicks and is not focused.
    let disabled_clicks = click_recording_targets(&mut harness, "#l_for_dis", None);
    assert_eq!(disabled_clicks, vec![l_for_dis]);
    assert_ne!(harness.focused(), Some(btn_disabled));

    let fieldset_clicks = click_recording_targets(&mut harness, "#fieldset_caption", None);
    assert_eq!(fieldset_clicks, vec![fieldset_caption]);
    assert_ne!(harness.focused(), Some(btn_fieldset_dis));

    // 4. Checkbox bound to <label> toggles once (no double-toggle) and respects prevent_default() on synthetic Click.
    assert!(!is_checked(&harness, "#cb_single"));
    let cb_clicks = click_recording_targets(&mut harness, "#l_for_cb", None);
    assert_eq!(cb_clicks, vec![l_for_cb, cb_single]);
    assert!(is_checked(&harness, "#cb_single"));
    assert_eq!(harness.focused(), Some(cb_single));

    let cancelled_cb_clicks = click_recording_targets(&mut harness, "#l_for_cb", Some(cb_single));
    assert_eq!(cancelled_cb_clicks, vec![l_for_cb, cb_single]);
    assert!(is_checked(&harness, "#cb_single"));
}
