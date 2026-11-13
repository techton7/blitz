//! Exhaustive HTML keyboard activation (`Enter` and `Space`) integration suite
//! covering `DioxusLabs/blitz#839` and Sections 3.2–3.4 of the Keyboard Activation
//! Feature Ledger.

use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use blitz_dom::{
    Document, DocumentConfig, ElementState, EventDriver, EventHandler, NodeId, QualName,
    local_name, ns,
};
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_test_harness::Harness;
use blitz_traits::{
    events::{
        BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, DomEvent, DomEventData, EventState,
        KeyState, MouseEventButton, MouseEventButtons, Point, PointerCoords, PointerDetails,
        UiEvent,
    },
    navigation::{NavigationOptions, NavigationProvider},
    net::{Body, EntryValue},
    shell::{ColorScheme, FileDialogFilter, ShellProvider, Viewport},
};
use dioxus::prelude::*;
use keyboard_types::{Code, Key, Location, Modifiers, NamedKey};

#[derive(Default)]
struct RecordingNav {
    requests: Mutex<Vec<NavigationOptions>>,
}

impl NavigationProvider for RecordingNav {
    fn navigate_to(&self, options: NavigationOptions) {
        self.requests.lock().unwrap().push(options);
    }
}

impl RecordingNav {
    fn take(&self) -> Vec<NavigationOptions> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

#[derive(Default)]
struct RecordingShell {
    redraws: AtomicUsize,
    file_dialogs: AtomicUsize,
    files_to_return: Mutex<Vec<PathBuf>>,
}

impl ShellProvider for RecordingShell {
    fn request_redraw(&self) {
        self.redraws.fetch_add(1, Ordering::SeqCst);
    }

    fn open_file_dialog(&self, _multiple: bool, _filter: Option<FileDialogFilter>) -> Vec<PathBuf> {
        self.file_dialogs.fetch_add(1, Ordering::SeqCst);
        self.files_to_return.lock().unwrap().clone()
    }
}

impl RecordingShell {
    fn take_redraws(&self) -> usize {
        self.redraws.swap(0, Ordering::SeqCst)
    }

    fn take_file_dialogs(&self) -> usize {
        self.file_dialogs.swap(0, Ordering::SeqCst)
    }
}

#[derive(Default, Clone)]
struct RecordedEvent {
    target: NodeId,
    kind: &'static str,
    mods: Modifiers,
}

struct RecordingHandler {
    events: Vec<RecordedEvent>,
    cancel_keydown_for: Option<Key>,
    cancel_keyup_for: Option<Key>,
    cancel_click: bool,
}

impl Default for RecordingHandler {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            cancel_keydown_for: None,
            cancel_keyup_for: None,
            cancel_click: false,
        }
    }
}

impl RecordingHandler {
    fn clicks_on(&self, id: NodeId) -> usize {
        self.events
            .iter()
            .filter(|e| e.kind == "click" && e.target == id)
            .count()
    }

    fn total_clicks(&self) -> usize {
        self.events.iter().filter(|e| e.kind == "click").count()
    }

    fn inputs_on(&self, id: NodeId) -> usize {
        self.events
            .iter()
            .filter(|e| e.kind == "input" && e.target == id)
            .count()
    }

    fn dblclicks_on(&self, id: NodeId) -> usize {
        self.events
            .iter()
            .filter(|e| e.kind == "dblclick" && e.target == id)
            .count()
    }

    fn last_click_mods(&self, id: NodeId) -> Option<Modifiers> {
        self.events
            .iter()
            .rev()
            .find(|e| e.kind == "click" && e.target == id)
            .map(|e| e.mods)
    }

    fn clear(&mut self) {
        self.events.clear();
    }
}

impl EventHandler for &mut RecordingHandler {
    fn handle_event(
        &mut self,
        _chain: &[NodeId],
        event: &mut DomEvent,
        _doc: &mut dyn Document,
        event_state: &mut EventState,
    ) {
        match &event.data {
            DomEventData::KeyDown(ke) => {
                self.events.push(RecordedEvent {
                    target: event.target,
                    kind: "keydown",
                    mods: ke.modifiers,
                });
                if self.cancel_keydown_for.as_ref() == Some(&ke.key) {
                    event_state.prevent_default();
                }
            }
            DomEventData::KeyUp(ke) => {
                self.events.push(RecordedEvent {
                    target: event.target,
                    kind: "keyup",
                    mods: ke.modifiers,
                });
                if self.cancel_keyup_for.as_ref() == Some(&ke.key) {
                    event_state.prevent_default();
                }
            }
            DomEventData::Click(pe) => {
                self.events.push(RecordedEvent {
                    target: event.target,
                    kind: "click",
                    mods: pe.mods,
                });
                if self.cancel_click {
                    event_state.prevent_default();
                }
            }
            DomEventData::Input(_) => {
                self.events.push(RecordedEvent {
                    target: event.target,
                    kind: "input",
                    mods: Modifiers::empty(),
                });
            }
            DomEventData::DoubleClick(pe) => {
                self.events.push(RecordedEvent {
                    target: event.target,
                    kind: "dblclick",
                    mods: pe.mods,
                });
            }
            _ => {}
        }
    }
}

fn make_doc(html: &str) -> (HtmlDocument, Arc<RecordingNav>, Arc<RecordingShell>) {
    let nav = Arc::new(RecordingNav::default());
    let shell = Arc::new(RecordingShell::default());
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
            base_url: Some("https://example.com/page".to_string()),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            navigation_provider: Some(nav.clone() as _),
            shell_provider: Some(shell.clone() as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    (doc, nav, shell)
}

fn sel(doc: &HtmlDocument, selector: &str) -> NodeId {
    doc.query_selector(selector).unwrap().expect(selector)
}

fn make_key(
    key: Key,
    code: Code,
    state: KeyState,
    modifiers: Modifiers,
    is_auto_repeating: bool,
    is_composing: bool,
) -> BlitzKeyEvent {
    let text = match (&key, state) {
        (Key::Character(s), KeyState::Pressed) => Some(s.as_str().into()),
        _ => None,
    };
    BlitzKeyEvent {
        key,
        code,
        modifiers,
        location: Location::Standard,
        is_auto_repeating,
        is_composing,
        state,
        text,
    }
}

fn enter_down() -> UiEvent {
    UiEvent::KeyDown(make_key(
        Key::Named(NamedKey::Enter),
        Code::Enter,
        KeyState::Pressed,
        Modifiers::empty(),
        false,
        false,
    ))
}

fn enter_up() -> UiEvent {
    UiEvent::KeyUp(make_key(
        Key::Named(NamedKey::Enter),
        Code::Enter,
        KeyState::Released,
        Modifiers::empty(),
        false,
        false,
    ))
}

fn space_down() -> UiEvent {
    UiEvent::KeyDown(make_key(
        Key::Character(" ".to_string()),
        Code::Space,
        KeyState::Pressed,
        Modifiers::empty(),
        false,
        false,
    ))
}

fn space_up() -> UiEvent {
    UiEvent::KeyUp(make_key(
        Key::Character(" ".to_string()),
        Code::Space,
        KeyState::Released,
        Modifiers::empty(),
        false,
        false,
    ))
}

fn escape_down() -> UiEvent {
    UiEvent::KeyDown(make_key(
        Key::Named(NamedKey::Escape),
        Code::Escape,
        KeyState::Pressed,
        Modifiers::empty(),
        false,
        false,
    ))
}

fn tab_down(shift: bool) -> UiEvent {
    let mods = if shift {
        Modifiers::SHIFT
    } else {
        Modifiers::empty()
    };
    UiEvent::KeyDown(make_key(
        Key::Named(NamedKey::Tab),
        Code::Tab,
        KeyState::Pressed,
        mods,
        false,
        false,
    ))
}

fn drive(doc: &mut HtmlDocument, handler: &mut RecordingHandler, event: UiEvent) {
    let mut driver = EventDriver::new(doc, handler);
    driver.handle_ui_event(event);
}

fn press_enter(doc: &mut HtmlDocument, handler: &mut RecordingHandler) {
    drive(doc, handler, enter_down());
    drive(doc, handler, enter_up());
    doc.resolve(0.0);
}

fn press_space(doc: &mut HtmlDocument, handler: &mut RecordingHandler) {
    drive(doc, handler, space_down());
    drive(doc, handler, space_up());
    doc.resolve(0.0);
}

fn mouse_event(x: f32, y: f32) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons: MouseEventButtons::from(MouseEventButton::Main),
        mods: Modifiers::empty(),
        details: PointerDetails::default(),
        element: Point::default(),
        active_pointers: Default::default(),
    }
}

fn node_center(doc: &HtmlDocument, id: NodeId) -> (f32, f32) {
    let n = doc.get_node(id).unwrap();
    let pos = n.absolute_position(0.0, 0.0);
    let size = n.final_layout().size;
    (pos.x + size.width / 2.0, pos.y + size.height / 2.0)
}

fn state_of(doc: &HtmlDocument, id: NodeId) -> ElementState {
    *doc.get_node(id).unwrap().element_state()
}

// ============================================================================
// Section 3.2: HTML Element & Input Type Matrix
// ============================================================================

#[test]
fn button_submit_enter_and_space() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/submit">
                <input type="text" name="q" value="ok">
                <button id="omitted">Omitted</button>
                <button id="explicit" type="submit">Explicit</button>
                <button id="invalid" type="bogus">Invalid</button>
                <button id="cmd" command="--open">Command</button>
            </form>
            <button id="standalone">Standalone</button>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();

    for selector in ["#omitted", "#explicit", "#invalid"] {
        let id = sel(&doc, selector);
        doc.set_focus_to(id);
        press_enter(&mut doc, &mut h);
        assert_eq!(h.clicks_on(id), 1, "{selector} Enter click");
        assert_eq!(nav.take().len(), 1, "{selector} Enter form submit");

        press_space(&mut doc, &mut h);
        assert_eq!(h.clicks_on(id), 2, "{selector} Space click");
        assert_eq!(nav.take().len(), 1, "{selector} Space form submit");
    }

    // <button command="..."> without type defaults to non-submit per HTML §4.10.6
    let cmd = sel(&doc, "#cmd");
    doc.set_focus_to(cmd);
    press_enter(&mut doc, &mut h);
    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(cmd), 2);
    assert_eq!(nav.take().len(), 0);

    // Standalone <button> outside <form> fires Click without navigation
    let standalone = sel(&doc, "#standalone");
    doc.set_focus_to(standalone);
    press_enter(&mut doc, &mut h);
    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(standalone), 2);
    assert_eq!(nav.take().len(), 0);
}

#[test]
fn button_type_button_no_submit() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/submit">
                <button id="btn" type="button">Action</button>
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");
    doc.set_focus_to(btn);

    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(btn), 1);
    assert!(nav.take().is_empty());

    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(btn), 2);
    assert!(nav.take().is_empty());
}

#[test]
fn button_type_reset_click_no_submit() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/submit">
                <button id="rst" type="reset">Reset</button>
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let rst = sel(&doc, "#rst");
    doc.set_focus_to(rst);

    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(rst), 1);
    assert!(nav.take().is_empty());

    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(rst), 2);
    assert!(nav.take().is_empty());
}

#[test]
fn input_submit_and_image_activation() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/submit">
                <input id="sub" type="submit" value="Send">
                <input id="img" type="image" alt="Go">
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();

    for selector in ["#sub", "#img"] {
        let id = sel(&doc, selector);
        doc.set_focus_to(id);

        press_enter(&mut doc, &mut h);
        assert_eq!(h.clicks_on(id), 1, "{selector} Enter click");
        assert_eq!(nav.take().len(), 1, "{selector} Enter submit");

        press_space(&mut doc, &mut h);
        assert_eq!(h.clicks_on(id), 2, "{selector} Space click");
        assert_eq!(nav.take().len(), 1, "{selector} Space submit");
    }
}

#[test]
fn input_type_button_activation() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/submit">
                <input type="text" name="only_one" value="1">
                <input id="ibtn" type="button" value="Push">
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let ibtn = sel(&doc, "#ibtn");
    doc.set_focus_to(ibtn);

    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(ibtn), 1);
    assert!(nav.take().is_empty());

    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(ibtn), 2);
    assert!(nav.take().is_empty());
}

#[test]
fn input_type_reset_activation() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/submit">
                <input id="irst" type="reset" value="Clear">
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let irst = sel(&doc, "#irst");
    doc.set_focus_to(irst);

    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(irst), 1);
    assert!(nav.take().is_empty());

    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(irst), 2);
    assert!(nav.take().is_empty());
}

#[test]
fn input_file_keyboard_click() {
    let (mut doc, _, shell) = make_doc(r#"<html><body><input id="f" type="file"></body></html>"#);
    *shell.files_to_return.lock().unwrap() = vec![PathBuf::from("/tmp/report.txt")];
    let mut h = RecordingHandler::default();
    let f = sel(&doc, "#f");
    doc.set_focus_to(f);

    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(f), 1);
    assert_eq!(shell.take_file_dialogs(), 1);

    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(f), 2);
    assert_eq!(shell.take_file_dialogs(), 1);
}

#[test]
fn checkbox_space_toggles_enter_does_not() {
    let (mut doc, _, _) = make_doc(r#"<html><body><input id="chk" type="checkbox"></body></html>"#);
    let mut h = RecordingHandler::default();
    let chk = sel(&doc, "#chk");
    doc.set_focus_to(chk);

    let is_checked = |doc: &HtmlDocument| {
        doc.get_node(chk)
            .unwrap()
            .element_data()
            .unwrap()
            .checkbox_input_checked()
            .unwrap()
    };

    // Enter MUST NOT toggle or fire Click
    press_enter(&mut doc, &mut h);
    assert!(!is_checked(&doc));
    assert!(!state_of(&doc, chk).contains(ElementState::CHECKED));
    assert_eq!(h.clicks_on(chk), 0);
    assert_eq!(h.inputs_on(chk), 0);

    // Space KeyDown arms :active without toggling yet
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, chk).contains(ElementState::ACTIVE));
    assert!(!is_checked(&doc));
    assert_eq!(h.clicks_on(chk), 0);

    // Space KeyUp dispatches Click + Input and toggles checked state
    drive(&mut doc, &mut h, space_up());
    doc.resolve(0.0);
    assert!(!state_of(&doc, chk).contains(ElementState::ACTIVE));
    assert!(is_checked(&doc));
    assert!(state_of(&doc, chk).contains(ElementState::CHECKED));
    assert_eq!(h.clicks_on(chk), 1);
    assert_eq!(h.inputs_on(chk), 1);

    // Cancelling Click prevents toggle
    h.cancel_click = true;
    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(chk), 2);
    assert!(is_checked(&doc));
}

#[test]
fn radio_space_checks_enter_does_not() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body>
            <input id="r1" type="radio" name="grp" checked>
            <input id="r2" type="radio" name="grp">
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let r1 = sel(&doc, "#r1");
    let r2 = sel(&doc, "#r2");
    doc.set_focus_to(r2);

    let checked = |doc: &HtmlDocument, id: NodeId| {
        doc.get_node(id)
            .unwrap()
            .element_data()
            .unwrap()
            .checkbox_input_checked()
            .unwrap()
    };

    // Enter MUST NOT check r2 or synthesize Click
    press_enter(&mut doc, &mut h);
    assert!(checked(&doc, r1));
    assert!(!checked(&doc, r2));
    assert_eq!(h.clicks_on(r2), 0);

    // Space checks r2 and unchecks r1
    press_space(&mut doc, &mut h);
    assert!(!checked(&doc, r1));
    assert!(checked(&doc, r2));
    assert_eq!(h.clicks_on(r2), 1);
    assert_eq!(h.inputs_on(r2), 1);
}

#[test]
fn text_input_space_types_enter_submits_form() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/search">
                <input id="txt" type="text" name="q">
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let txt = sel(&doc, "#txt");
    doc.set_focus_to(txt);

    // Space inserts " " into TextInputData without arming :active or synthesizing Click
    drive(&mut doc, &mut h, space_down());
    assert!(!state_of(&doc, txt).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    doc.resolve(0.0);

    let val = doc
        .get_node(txt)
        .unwrap()
        .element_data()
        .unwrap()
        .text_input_data()
        .unwrap()
        .editor
        .raw_text()
        .to_string();
    assert_eq!(val, " ");
    assert_eq!(h.clicks_on(txt), 0);
    assert_eq!(h.inputs_on(txt), 1);
    assert!(nav.take().is_empty());

    // Enter submits the form implicitly without synthesizing Click on #txt
    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(txt), 0);
    assert_eq!(nav.take().len(), 1);
}

#[test]
fn textarea_enter_newline_space_types() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/post">
                <textarea id="ta" name="body"></textarea>
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let ta = sel(&doc, "#ta");
    doc.set_focus_to(ta);

    press_space(&mut doc, &mut h);
    press_enter(&mut doc, &mut h);

    let val = doc
        .get_node(ta)
        .unwrap()
        .element_data()
        .unwrap()
        .text_input_data()
        .unwrap()
        .editor
        .raw_text()
        .to_string();
    assert_eq!(val, " \n");
    assert_eq!(h.clicks_on(ta), 0);
    assert!(nav.take().is_empty());
}

#[test]
fn link_enter_activates_space_does_not() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <a id="lnk" href="https://example.com/dest">Destination</a>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let lnk = sel(&doc, "#lnk");
    doc.set_focus_to(lnk);

    // Space MUST NOT arm :active, fire Click, or navigate
    drive(&mut doc, &mut h, space_down());
    assert!(!state_of(&doc, lnk).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(lnk), 0);
    assert!(nav.take().is_empty());

    // Enter activates on KeyDown
    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(lnk), 1);
    let navs = nav.take();
    assert_eq!(navs.len(), 1);
    assert_eq!(navs[0].url.as_str(), "https://example.com/dest");
}

#[test]
fn summary_first_child_enter_and_space() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body>
            <details id="det">
                <summary id="sum">Toggle</summary>
                <p>Body</p>
            </details>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let det = sel(&doc, "#det");
    let sum = sel(&doc, "#sum");
    doc.set_focus_to(sum);

    assert!(
        !doc.get_node(det)
            .unwrap()
            .data
            .has_attr(local_name!("open"))
    );

    // Enter opens <details>
    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(sum), 1);
    assert!(
        doc.get_node(det)
            .unwrap()
            .data
            .has_attr(local_name!("open"))
    );

    // Space closes <details>
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, sum).contains(ElementState::ACTIVE));
    assert!(
        doc.get_node(det)
            .unwrap()
            .data
            .has_attr(local_name!("open"))
    );

    drive(&mut doc, &mut h, space_up());
    doc.resolve(0.0);
    assert!(!state_of(&doc, sum).contains(ElementState::ACTIVE));
    assert_eq!(h.clicks_on(sum), 2);
    assert!(
        !doc.get_node(det)
            .unwrap()
            .data
            .has_attr(local_name!("open"))
    );
}

#[test]
fn non_interactive_anchor_and_secondary_summary_inert() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <a id="nohref" tabindex="0">Anchor without href</a>
            <area id="areanohref" tabindex="0">
            <summary id="orphan" tabindex="0">Orphan summary</summary>
            <details id="det" open>
                <summary id="first">First</summary>
                <summary id="second" tabindex="0">Second</summary>
            </details>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let det = sel(&doc, "#det");

    for selector in ["#nohref", "#areanohref", "#orphan", "#second"] {
        let id = sel(&doc, selector);
        doc.set_focus_to(id);
        press_enter(&mut doc, &mut h);
        press_space(&mut doc, &mut h);
        assert_eq!(h.clicks_on(id), 0, "{selector} must not synthesize Click");
    }
    assert!(
        doc.get_node(det)
            .unwrap()
            .data
            .has_attr(local_name!("open"))
    );
    assert!(nav.take().is_empty());
}

#[test]
fn div_role_button_not_engine_activated() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body>
            <div id="rbtn" role="button" tabindex="0">Pseudo Button</div>
            <span id="spn" tabindex="0">Focusable Span</span>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();

    for selector in ["#rbtn", "#spn"] {
        let id = sel(&doc, selector);
        doc.set_focus_to(id);
        press_enter(&mut doc, &mut h);
        press_space(&mut doc, &mut h);
        assert_eq!(
            h.clicks_on(id),
            0,
            "{selector} must not be engine-activated"
        );
    }
}

// ============================================================================
// Section 3.3: Key Timing, Modifier, Auto-Repeat & State-Machine Ledger
// ============================================================================

#[test]
fn nested_input_in_label_or_link_does_not_activate_ancestor() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <a id="anc_link" href="https://example.com/jump">
                <input id="inner_txt" type="text">
                <button id="inner_btn" type="button">Inner Button</button>
            </a>
            <label id="lbl" for="bound_chk">
                <input id="label_txt" type="text">
            </label>
            <input id="bound_chk" type="checkbox">
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let anc_link = sel(&doc, "#anc_link");
    let inner_txt = sel(&doc, "#inner_txt");
    let inner_btn = sel(&doc, "#inner_btn");
    let label_txt = sel(&doc, "#label_txt");
    let bound_chk = sel(&doc, "#bound_chk");

    // Focused text input inside <a href>: Space types, Enter does not click/navigate <a href>
    doc.set_focus_to(inner_txt);
    press_space(&mut doc, &mut h);
    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(anc_link), 0);
    assert!(nav.take().is_empty());

    // Focused <button type="button"> inside <a href>: clicks button, does NOT navigate <a href>
    doc.set_focus_to(inner_btn);
    press_enter(&mut doc, &mut h);
    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(inner_btn), 2);
    assert!(nav.take().is_empty());

    // Focused text input inside <label>: Space/Enter do not toggle bound checkbox
    doc.set_focus_to(label_txt);
    press_space(&mut doc, &mut h);
    press_enter(&mut doc, &mut h);
    assert!(!state_of(&doc, bound_chk).contains(ElementState::CHECKED));

    // Unfocused document (root fallback): Enter/Space fire zero clicks
    doc.clear_focus();
    h.clear();
    press_enter(&mut doc, &mut h);
    press_space(&mut doc, &mut h);
    assert_eq!(h.total_clicks(), 0);
}

#[test]
fn ime_composing_enter_space_ignored() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/ime">
                <input id="txt" type="text" name="q" value="a">
            </form>
            <button id="btn" type="button">Btn</button>
            <a id="lnk" href="https://example.com/go">Link</a>
            <input id="chk" type="checkbox">
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();

    for selector in ["#txt", "#btn", "#lnk", "#chk"] {
        let id = sel(&doc, selector);
        doc.set_focus_to(id);
        drive(
            &mut doc,
            &mut h,
            UiEvent::KeyDown(make_key(
                Key::Named(NamedKey::Enter),
                Code::Enter,
                KeyState::Pressed,
                Modifiers::empty(),
                false,
                true,
            )),
        );
        drive(
            &mut doc,
            &mut h,
            UiEvent::KeyDown(make_key(
                Key::Character(" ".to_string()),
                Code::Space,
                KeyState::Pressed,
                Modifiers::empty(),
                false,
                true,
            )),
        );
        assert!(!state_of(&doc, id).contains(ElementState::ACTIVE));
        drive(
            &mut doc,
            &mut h,
            UiEvent::KeyUp(make_key(
                Key::Character(" ".to_string()),
                Code::Space,
                KeyState::Released,
                Modifiers::empty(),
                false,
                true,
            )),
        );
    }
    assert_eq!(h.total_clicks(), 0);
    assert!(nav.take().is_empty());
}

#[test]
fn enter_fires_on_keydown_not_keyup() {
    let (mut doc, _, _) = make_doc(r#"<html><body><button id="btn">Btn</button></body></html>"#);
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");
    doc.set_focus_to(btn);

    // Standalone KeyUp(Enter) is a no-op
    drive(&mut doc, &mut h, enter_up());
    assert_eq!(h.clicks_on(btn), 0);

    // KeyDown(Enter) immediately synthesizes Click before KeyUp arrives
    drive(&mut doc, &mut h, enter_down());
    assert_eq!(h.clicks_on(btn), 1);

    // Subsequent KeyUp(Enter) does not fire a second Click
    drive(&mut doc, &mut h, enter_up());
    assert_eq!(h.clicks_on(btn), 1);
}

#[test]
fn enter_modifier_filtering() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <button id="btn" type="button">Btn</button>
            <a id="lnk" href="https://example.com/dest">Link</a>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");
    let lnk = sel(&doc, "#lnk");

    // Button: Shift+Enter allowed, Ctrl/Alt/Meta rejected
    doc.set_focus_to(btn);
    for blocked_mod in [Modifiers::CONTROL, Modifiers::ALT, Modifiers::META] {
        drive(
            &mut doc,
            &mut h,
            UiEvent::KeyDown(make_key(
                Key::Named(NamedKey::Enter),
                Code::Enter,
                KeyState::Pressed,
                blocked_mod,
                false,
                false,
            )),
        );
        assert_eq!(
            h.clicks_on(btn),
            0,
            "button must reject {blocked_mod:?}+Enter"
        );
    }

    drive(
        &mut doc,
        &mut h,
        UiEvent::KeyDown(make_key(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Pressed,
            Modifiers::SHIFT,
            false,
            false,
        )),
    );
    assert_eq!(h.clicks_on(btn), 1);
    assert_eq!(h.last_click_mods(btn), Some(Modifiers::SHIFT));

    // Link: Shift, Ctrl, Meta allowed (and forwarded); Alt rejected
    doc.set_focus_to(lnk);
    drive(
        &mut doc,
        &mut h,
        UiEvent::KeyDown(make_key(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Pressed,
            Modifiers::ALT,
            false,
            false,
        )),
    );
    assert_eq!(h.clicks_on(lnk), 0);
    assert!(nav.take().is_empty());

    for allowed_mod in [Modifiers::SHIFT, Modifiers::CONTROL, Modifiers::META] {
        drive(
            &mut doc,
            &mut h,
            UiEvent::KeyDown(make_key(
                Key::Named(NamedKey::Enter),
                Code::Enter,
                KeyState::Pressed,
                allowed_mod,
                false,
                false,
            )),
        );
        assert_eq!(h.last_click_mods(lnk), Some(allowed_mod));
    }
    assert_eq!(h.clicks_on(lnk), 3);
    assert_eq!(nav.take().len(), 3);
}

#[test]
fn enter_auto_repeat_fires_once() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body>
            <details id="det"><summary id="sum">Sum</summary><p>Body</p></details>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let det = sel(&doc, "#det");
    let sum = sel(&doc, "#sum");
    doc.set_focus_to(sum);

    // Initial press opens details
    drive(&mut doc, &mut h, enter_down());
    assert_eq!(h.clicks_on(sum), 1);
    assert!(
        doc.get_node(det)
            .unwrap()
            .data
            .has_attr(local_name!("open"))
    );

    // Auto-repeating KeyDown(Enter) must NOT toggle back closed
    for _ in 0..3 {
        drive(
            &mut doc,
            &mut h,
            UiEvent::KeyDown(make_key(
                Key::Named(NamedKey::Enter),
                Code::Enter,
                KeyState::Pressed,
                Modifiers::empty(),
                true,
                false,
            )),
        );
    }
    assert_eq!(h.clicks_on(sum), 1);
    assert!(
        doc.get_node(det)
            .unwrap()
            .data
            .has_attr(local_name!("open"))
    );
}

#[test]
fn space_keydown_arms_active_without_clicking() {
    let (mut doc, _, shell) = make_doc(
        r#"<html><body>
            <div id="wrap"><button id="btn" type="button" style="width:80px;height:30px;">Btn</button></div>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let wrap = sel(&doc, "#wrap");
    let btn = sel(&doc, "#btn");
    doc.set_focus_to(btn);
    shell.take_redraws();

    // Modified Space does NOT arm :active
    for m in [
        Modifiers::SHIFT,
        Modifiers::CONTROL,
        Modifiers::ALT,
        Modifiers::META,
    ] {
        drive(
            &mut doc,
            &mut h,
            UiEvent::KeyDown(make_key(
                Key::Character(" ".to_string()),
                Code::Space,
                KeyState::Pressed,
                m,
                false,
                false,
            )),
        );
        assert!(!state_of(&doc, btn).contains(ElementState::ACTIVE));
    }

    // Unmodified Space KeyDown arms :active on button and ancestors, requests redraw, zero Click
    shell.take_redraws();
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, btn).contains(ElementState::ACTIVE));
    assert!(state_of(&doc, wrap).contains(ElementState::ACTIVE));
    assert!(shell.take_redraws() >= 1);
    assert_eq!(h.clicks_on(btn), 0);

    // Pointer press + release while keyboard Space is held must not wipe keyboard :active
    let (cx, cy) = node_center(&doc, btn);
    doc.set_hover_to(cx, cy);
    doc.active_node();
    doc.unactive_node();
    assert!(
        state_of(&doc, btn).contains(ElementState::ACTIVE),
        "pointer unactive_node must preserve ElementState::ACTIVE while keyboard Space is armed"
    );

    drive(&mut doc, &mut h, space_up());
    assert!(!state_of(&doc, btn).contains(ElementState::ACTIVE));
    assert!(!state_of(&doc, wrap).contains(ElementState::ACTIVE));
    assert_eq!(h.clicks_on(btn), 1);
}

#[test]
fn space_hold_escape_autorepeat_stays_disarmed() {
    let (mut doc, _, _) =
        make_doc(r#"<html><body><button id="btn" type="button">Btn</button></body></html>"#);
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");
    // Prevent Escape default clear_focus so #btn remains focused while testing auto-repeat guard
    h.cancel_keydown_for = Some(Key::Named(NamedKey::Escape));
    doc.set_focus_to(btn);

    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, btn).contains(ElementState::ACTIVE));

    // Escape disarms even when Escape preventDefault() is called
    drive(&mut doc, &mut h, escape_down());
    assert!(!state_of(&doc, btn).contains(ElementState::ACTIVE));
    assert_eq!(doc.get_focussed_node_id(), Some(btn));

    // Subsequent auto-repeat Space KeyDown must NOT re-arm
    drive(
        &mut doc,
        &mut h,
        UiEvent::KeyDown(make_key(
            Key::Character(" ".to_string()),
            Code::Space,
            KeyState::Pressed,
            Modifiers::empty(),
            true,
            false,
        )),
    );
    assert!(!state_of(&doc, btn).contains(ElementState::ACTIVE));

    // Releasing Space fires zero Click
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(btn), 0);
}

#[test]
fn dioxus_keydown_prevent_default_blocks_space_and_enter_click() {
    static PATTERN_B_KEYDOWN: AtomicUsize = AtomicUsize::new(0);
    static PATTERN_B_CLICK: AtomicUsize = AtomicUsize::new(0);
    static PATTERN_A_CLICK: AtomicUsize = AtomicUsize::new(0);

    PATTERN_B_KEYDOWN.store(0, Ordering::SeqCst);
    PATTERN_B_CLICK.store(0, Ordering::SeqCst);
    PATTERN_A_CLICK.store(0, Ordering::SeqCst);

    fn app() -> Element {
        rsx! {
            div {
                button {
                    id: "pattern-b",
                    r#type: "button",
                    onkeydown: move |evt| {
                        if evt.key() == Key::Named(NamedKey::Enter)
                            || evt.key() == Key::Character(" ".to_string())
                        {
                            evt.prevent_default();
                            PATTERN_B_KEYDOWN.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                    onclick: move |_| {
                        PATTERN_B_CLICK.fetch_add(1, Ordering::SeqCst);
                    },
                    "Pattern B"
                }
                button {
                    id: "pattern-a",
                    r#type: "button",
                    onclick: move |_| {
                        PATTERN_A_CLICK.fetch_add(1, Ordering::SeqCst);
                    },
                    "Pattern A"
                }
            }
        }
    }

    let mut harness = Harness::from_component(app);
    let b_id = harness.node("#pattern-b");
    let a_id = harness.node("#pattern-a");

    // Pattern B: onkeydown calls evt.prevent_default() -> zero synthetic onclick double-fire
    harness.base_mut().set_focus_to(b_id);
    harness.press(Key::Named(NamedKey::Enter));
    harness.press(Key::Character(" ".to_string()));
    assert_eq!(PATTERN_B_KEYDOWN.load(Ordering::SeqCst), 2);
    assert_eq!(PATTERN_B_CLICK.load(Ordering::SeqCst), 0);
    assert!(
        !harness
            .base()
            .get_node(b_id)
            .unwrap()
            .element_state()
            .contains(ElementState::ACTIVE)
    );

    // Pattern A: plain <button> with onclick only -> Enter and Space each fire onclick once
    harness.base_mut().set_focus_to(a_id);
    harness.press(Key::Named(NamedKey::Enter));
    assert_eq!(PATTERN_A_CLICK.load(Ordering::SeqCst), 1);
    harness.press(Key::Character(" ".to_string()));
    assert_eq!(PATTERN_A_CLICK.load(Ordering::SeqCst), 2);
}

#[test]
fn space_cancelled_by_escape_including_prevent_default() {
    let (mut doc, _, _) =
        make_doc(r#"<html><body><button id="btn" type="button">Btn</button></body></html>"#);
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");

    // Case 1: Normal Escape (clears focus and disarms)
    doc.set_focus_to(btn);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, btn).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, escape_down());
    assert!(!state_of(&doc, btn).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(btn), 0);

    // Case 2: Overlay keydown listener calls preventDefault() on Escape -> still disarms
    h.cancel_keydown_for = Some(Key::Named(NamedKey::Escape));
    doc.set_focus_to(btn);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, btn).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, escape_down());
    assert!(!state_of(&doc, btn).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(btn), 0);
}

#[test]
fn space_cancelled_by_blur_and_tab_focus_move() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body>
            <button id="b1" type="button">One</button>
            <button id="b2" type="button">Two</button>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let b1 = sel(&doc, "#b1");
    let b2 = sel(&doc, "#b2");

    // Tab while holding Space on #b1 moves focus to #b2, clears :active on #b1, and Space KeyUp clicks neither
    doc.set_focus_to(b1);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, b1).contains(ElementState::ACTIVE));

    drive(&mut doc, &mut h, tab_down(false));
    assert_eq!(doc.get_focussed_node_id(), Some(b2));
    assert!(!state_of(&doc, b1).contains(ElementState::ACTIVE));
    assert!(!state_of(&doc, b2).contains(ElementState::ACTIVE));

    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(b1), 0);
    assert_eq!(h.clicks_on(b2), 0);

    // Programmatic blur (clear_focus) while holding Space also disarms
    doc.set_focus_to(b1);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, b1).contains(ElementState::ACTIVE));
    doc.clear_focus();
    assert!(!state_of(&doc, b1).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.total_clicks(), 0);
}

#[test]
fn space_cancelled_when_disabled_or_removed_mid_hold() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body>
            <button id="b_self" type="button">Self Disable</button>
            <fieldset id="fs"><button id="b_fs" type="button">Fieldset Disable</button></fieldset>
            <button id="b_hide" type="button">Hide</button>
            <div id="wrap"><button id="b_rem" type="button">Remove</button></div>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let disabled_attr = QualName::new(None, ns!(), local_name!("disabled"));

    // (a) Self disabled mid-hold
    let b_self = sel(&doc, "#b_self");
    doc.set_focus_to(b_self);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, b_self).contains(ElementState::ACTIVE));
    doc.mutate()
        .set_attribute(b_self, disabled_attr.clone(), "");
    assert!(!state_of(&doc, b_self).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(b_self), 0);

    // (b) Ancestor <fieldset> disabled mid-hold
    let fs = sel(&doc, "#fs");
    let b_fs = sel(&doc, "#b_fs");
    doc.set_focus_to(b_fs);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, b_fs).contains(ElementState::ACTIVE));
    doc.mutate().set_attribute(fs, disabled_attr, "");
    assert!(!state_of(&doc, b_fs).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(b_fs), 0);

    // (c) Hidden via display:none mid-hold
    let b_hide = sel(&doc, "#b_hide");
    doc.set_focus_to(b_hide);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, b_hide).contains(ElementState::ACTIVE));
    doc.set_style_property(b_hide, "display", "none");
    doc.resolve(0.0);
    assert!(!state_of(&doc, b_hide).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.clicks_on(b_hide), 0);

    // (d) Removed from DOM mid-hold
    let wrap = sel(&doc, "#wrap");
    let b_rem = sel(&doc, "#b_rem");
    doc.set_focus_to(b_rem);
    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, wrap).contains(ElementState::ACTIVE));
    doc.mutate().remove_node(b_rem);
    assert!(!state_of(&doc, wrap).contains(ElementState::ACTIVE));
    drive(&mut doc, &mut h, space_up());
    assert_eq!(h.total_clicks(), 0);
}

#[test]
fn space_keyup_prevent_default_clears_active_no_click() {
    let (mut doc, _, _) =
        make_doc(r#"<html><body><button id="btn" type="button">Btn</button></body></html>"#);
    let mut h = RecordingHandler::default();
    h.cancel_keyup_for = Some(Key::Character(" ".to_string()));
    let btn = sel(&doc, "#btn");
    doc.set_focus_to(btn);

    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, btn).contains(ElementState::ACTIVE));

    drive(&mut doc, &mut h, space_up());
    assert!(
        !state_of(&doc, btn).contains(ElementState::ACTIVE),
        "ElementState::ACTIVE must clear even when KeyUp calls preventDefault()"
    );
    assert_eq!(h.clicks_on(btn), 0);
}

#[test]
fn space_fires_on_keyup_when_still_armed_and_focused() {
    let (mut doc, _, shell) =
        make_doc(r#"<html><body><button id="btn" type="button">Btn</button></body></html>"#);
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");
    doc.set_focus_to(btn);
    shell.take_redraws();

    drive(&mut doc, &mut h, space_down());
    assert!(state_of(&doc, btn).contains(ElementState::ACTIVE));
    assert!(shell.take_redraws() >= 1);
    assert_eq!(h.clicks_on(btn), 0);

    drive(&mut doc, &mut h, space_up());
    assert!(!state_of(&doc, btn).contains(ElementState::ACTIVE));
    assert!(shell.take_redraws() >= 1);
    assert_eq!(h.clicks_on(btn), 1);
}

// ============================================================================
// Section 3.4: Disabled Guards & Existing blitz-dom Co-Fixes 1–4
// ============================================================================

#[test]
fn detached_node_cannot_activate() {
    let (mut doc, _, _) =
        make_doc(r#"<html><body><button id="btn" type="button">Detached</button></body></html>"#);
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");

    // Remove from parent without dropping slab slot if referenced, or remove_node
    doc.mutate().remove_node(btn);
    assert!(!doc.set_focus_to(btn));

    let mut driver = EventDriver::new(&mut doc, &mut h);
    driver.handle_dom_event(DomEvent::new(
        btn,
        DomEventData::KeyDown(make_key(
            Key::Named(NamedKey::Enter),
            Code::Enter,
            KeyState::Pressed,
            Modifiers::empty(),
            false,
            false,
        )),
    ));
    drop(driver);
    assert_eq!(h.total_clicks(), 0);
}

#[test]
fn disabled_button_input_inert_anchor_ignores_disabled() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body style="margin:0">
            <button id="dbtn" disabled style="width:80px;height:30px;">Disabled Btn</button>
            <input id="dinp" type="submit" disabled style="width:80px;height:30px;">
            <a id="alink" href="https://example.com/anchor" disabled style="display:block;width:80px;height:30px;">Anchor</a>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let dbtn = sel(&doc, "#dbtn");
    let dinp = sel(&doc, "#dinp");
    let alink = sel(&doc, "#alink");

    // Tab skips disabled button and disabled input, landing directly on <a href disabled>
    assert_eq!(doc.focus_next_node(), Some(alink));

    // Even if focus is forced onto disabled controls, Enter/Space/pointer click are inert
    doc.set_focus_to(dbtn);
    press_enter(&mut doc, &mut h);
    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(dbtn), 0);

    doc.set_focus_to(dinp);
    press_enter(&mut doc, &mut h);
    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(dinp), 0);
    assert!(nav.take().is_empty());

    // <a href="..." disabled> is NOT disableable in HTML: activates on Enter AND pointer click
    doc.set_focus_to(alink);
    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(alink), 1);
    assert_eq!(nav.take().len(), 1);

    let (ax, ay) = node_center(&doc, alink);
    let pe = mouse_event(ax, ay);
    drive(&mut doc, &mut h, UiEvent::PointerDown(pe.clone()));
    drive(&mut doc, &mut h, UiEvent::PointerUp(pe));
    assert_eq!(nav.take().len(), 1);
}

#[test]
fn fieldset_disabled_blocks_controls_except_first_legend() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body style="margin:0">
            <form action="https://example.com/fs" method="post">
                <fieldset disabled>
                    <legend>
                        <button id="leg1_btn" type="submit" style="width:60px;height:24px;">Allowed</button>
                        <input id="leg1_in" type="text" name="allowed" value="yes">
                    </legend>
                    <legend>
                        <button id="leg2_btn" type="button" style="width:60px;height:24px;">Second Legend</button>
                    </legend>
                    <button id="body_btn" type="button" style="width:60px;height:24px;">Blocked</button>
                    <input id="body_in" type="text" name="blocked" value="no">
                </fieldset>
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let leg1_btn = sel(&doc, "#leg1_btn");
    let leg1_in = sel(&doc, "#leg1_in");
    let leg2_btn = sel(&doc, "#leg2_btn");
    let body_btn = sel(&doc, "#body_btn");
    let body_in = sel(&doc, "#body_in");

    assert!(!doc.is_form_control_disabled(leg1_btn));
    assert!(!doc.is_form_control_disabled(leg1_in));
    assert!(doc.is_form_control_disabled(leg2_btn));
    assert!(doc.is_form_control_disabled(body_btn));
    assert!(doc.is_form_control_disabled(body_in));

    // Tab traversal visits ONLY first-<legend> controls (wrapping back to leg1_btn)
    assert_eq!(doc.focus_next_node(), Some(leg1_btn));
    assert_eq!(doc.focus_next_node(), Some(leg1_in));
    assert_eq!(doc.focus_next_node(), Some(leg1_btn));

    // Blocked controls ignore Enter, Space, and pointer click default actions
    for blocked in [leg2_btn, body_btn] {
        doc.set_focus_to(blocked);
        press_enter(&mut doc, &mut h);
        press_space(&mut doc, &mut h);
        assert_eq!(h.clicks_on(blocked), 0);
    }

    // First-<legend> submit button activates via Enter and construct_entry_list excludes disabled fieldset controls
    doc.set_focus_to(leg1_btn);
    press_enter(&mut doc, &mut h);
    assert_eq!(h.clicks_on(leg1_btn), 1);
    let navs = nav.take();
    assert_eq!(navs.len(), 1);
    match &navs[0].document_resource {
        Body::Form(form_data) => {
            assert_eq!(form_data.0.len(), 1);
            assert_eq!(form_data.0[0].name, "allowed");
            assert_eq!(form_data.0[0].value, EntryValue::String("yes".to_string()));
        }
        other => panic!("expected Body::Form, got {other:?}"),
    }
}

#[test]
fn keyboard_activation_preserves_focus_visible_ring() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body style="margin:0">
            <input id="chk" type="checkbox" style="width:24px;height:24px;">
            <input id="rad" type="radio" name="g" style="width:24px;height:24px;">
            <details id="det"><summary id="sum" style="width:80px;height:24px;">Sum</summary><p>Body</p></details>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let chk = sel(&doc, "#chk");
    let rad = sel(&doc, "#rad");
    let sum = sel(&doc, "#sum");

    // Tab into #chk -> FOCUSRING set -> Space toggles -> FOCUSRING preserved
    drive(&mut doc, &mut h, tab_down(false));
    assert_eq!(doc.get_focussed_node_id(), Some(chk));
    assert!(state_of(&doc, chk).contains(ElementState::FOCUSRING));
    press_space(&mut doc, &mut h);
    assert!(
        state_of(&doc, chk).contains(ElementState::FOCUSRING),
        "keyboard Space on checkbox must preserve :focus-visible FOCUSRING"
    );

    // Tab into #rad -> FOCUSRING set -> Space checks -> FOCUSRING preserved
    drive(&mut doc, &mut h, tab_down(false));
    assert_eq!(doc.get_focussed_node_id(), Some(rad));
    assert!(state_of(&doc, rad).contains(ElementState::FOCUSRING));
    press_space(&mut doc, &mut h);
    assert!(
        state_of(&doc, rad).contains(ElementState::FOCUSRING),
        "keyboard Space on radio must preserve :focus-visible FOCUSRING"
    );

    // Tab into #sum -> FOCUSRING set -> Enter & Space toggle -> FOCUSRING preserved
    drive(&mut doc, &mut h, tab_down(false));
    assert_eq!(doc.get_focussed_node_id(), Some(sum));
    assert!(state_of(&doc, sum).contains(ElementState::FOCUSRING));
    press_enter(&mut doc, &mut h);
    assert!(state_of(&doc, sum).contains(ElementState::FOCUSRING));
    press_space(&mut doc, &mut h);
    assert!(state_of(&doc, sum).contains(ElementState::FOCUSRING));

    // Contrast: mouse click on #chk clears FOCUSRING (FocusSource::Pointer)
    let (cx, cy) = node_center(&doc, chk);
    let pe = mouse_event(cx, cy);
    drive(&mut doc, &mut h, UiEvent::PointerDown(pe.clone()));
    drive(&mut doc, &mut h, UiEvent::PointerUp(pe));
    assert_eq!(doc.get_focussed_node_id(), Some(chk));
    assert!(!state_of(&doc, chk).contains(ElementState::FOCUSRING));
}

#[test]
fn keyboard_click_after_mouse_dblclick_no_spurious_dblclick() {
    let (mut doc, _, _) = make_doc(
        r#"<html><body style="margin:0">
            <button id="btn" type="button" style="width:80px;height:30px;">Btn</button>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();
    let btn = sel(&doc, "#btn");
    let (bx, by) = node_center(&doc, btn);
    let pe = mouse_event(bx, by);

    // Two quick mouse clicks produce 1 DoubleClick (leaving click_count == 2 if not reset)
    drive(&mut doc, &mut h, UiEvent::PointerDown(pe.clone()));
    drive(&mut doc, &mut h, UiEvent::PointerUp(pe.clone()));
    drive(&mut doc, &mut h, UiEvent::PointerDown(pe.clone()));
    drive(&mut doc, &mut h, UiEvent::PointerUp(pe));
    assert_eq!(h.clicks_on(btn), 2);
    assert_eq!(h.dblclicks_on(btn), 1);

    // Subsequent keyboard Enter and Space clicks must NOT emit spurious DoubleClick
    press_enter(&mut doc, &mut h);
    press_space(&mut doc, &mut h);
    assert_eq!(h.clicks_on(btn), 4);
    assert_eq!(h.dblclicks_on(btn), 1);
}

#[test]
fn area_href_case_insensitive_types_and_submit_defaults() {
    let (mut doc, nav, _) = make_doc(
        r#"<html><body>
            <map name="m"><area id="ar" href="https://example.com/area"></map>
            <input id="uchk" type="CHECKBOX">
            <input id="urad" type="RaDiO" name="rg">
            <form action="https://example.com/form_target">
                <a href="https://example.com/wrong_parent_link">
                    <button id="usub" type="SUBMIT">Submit</button>
                    <input id="uimg" type="IMAGE" alt="Img">
                </a>
            </form>
        </body></html>"#,
    );
    let mut h = RecordingHandler::default();

    // <area href="..."> activates on Enter
    let ar = sel(&doc, "#ar");
    doc.set_focus_to(ar);
    press_enter(&mut doc, &mut h);
    let navs = nav.take();
    assert_eq!(navs.len(), 1);
    assert_eq!(navs[0].url.as_str(), "https://example.com/area");

    // Case-insensitive CHECKBOX and RaDiO activate on Space
    let uchk = sel(&doc, "#uchk");
    doc.set_focus_to(uchk);
    press_space(&mut doc, &mut h);
    assert!(state_of(&doc, uchk).contains(ElementState::CHECKED));

    let urad = sel(&doc, "#urad");
    doc.set_focus_to(urad);
    press_space(&mut doc, &mut h);
    assert!(state_of(&doc, urad).contains(ElementState::CHECKED));

    // Case-insensitive SUBMIT & IMAGE inside <a href>: break 'matched prevents parent <a> navigation
    for selector in ["#usub", "#uimg"] {
        let id = sel(&doc, selector);
        doc.set_focus_to(id);
        press_enter(&mut doc, &mut h);
        let navs = nav.take();
        assert_eq!(
            navs.len(),
            1,
            "{selector} must submit form once without also navigating parent <a href>"
        );
        assert!(
            navs[0]
                .url
                .as_str()
                .starts_with("https://example.com/form_target")
        );
    }
}

#[test]
fn implicit_form_submit_omitted_type_and_submit_button_click() {
    // 1. Single <input> with omitted type (HTML default "text") and no submit button -> submits
    let (mut doc1, nav1, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/one" method="post">
                <input id="q" name="q" value="hello">
            </form>
        </body></html>"#,
    );
    let mut h1 = RecordingHandler::default();
    let q = sel(&doc1, "#q");
    doc1.set_focus_to(q);
    press_enter(&mut doc1, &mut h1);
    assert_eq!(nav1.take().len(), 1);

    // 2. Two <input>s with omitted type and NO submit button -> blocked (> 1 blocking field)
    let (mut doc2, nav2, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/two">
                <input id="q1" name="q1" value="a">
                <input id="q2" name="q2" value="b">
            </form>
        </body></html>"#,
    );
    let mut h2 = RecordingHandler::default();
    let q1 = sel(&doc2, "#q1");
    doc2.set_focus_to(q1);
    press_enter(&mut doc2, &mut h2);
    assert!(nav2.take().is_empty());

    // 3. Two <input>s WITH an enabled default submit button -> clicks default submit button
    let (mut doc3, nav3, _) = make_doc(
        r#"<html><body>
            <form action="https://example.com/three" method="post">
                <input id="f1" name="f1" value="a">
                <input id="f2" name="f2" value="b">
                <button id="def_btn" name="sub" value="clicked">Submit</button>
            </form>
        </body></html>"#,
    );
    let mut h3 = RecordingHandler::default();
    let f1 = sel(&doc3, "#f1");
    let def_btn = sel(&doc3, "#def_btn");
    doc3.set_focus_to(f1);

    // If onclick on default submit button calls preventDefault(), submission is cancelled
    h3.cancel_click = true;
    press_enter(&mut doc3, &mut h3);
    assert_eq!(h3.clicks_on(def_btn), 1);
    assert!(nav3.take().is_empty());

    // When onclick is not cancelled, default submit button submits form with itself as submitter
    h3.cancel_click = false;
    press_enter(&mut doc3, &mut h3);
    assert_eq!(h3.clicks_on(def_btn), 2);
    let navs = nav3.take();
    assert_eq!(navs.len(), 1);
    match &navs[0].document_resource {
        Body::Form(fd) => {
            assert!(fd.0.iter().any(|e| e.name == "sub"));
        }
        other => panic!("expected Body::Form, got {other:?}"),
    }

    // 4. Disabled default submit button -> HTML §4.10.21.2 does nothing
    doc3.mutate().set_attribute(
        def_btn,
        QualName::new(None, ns!(), local_name!("disabled")),
        "",
    );
    doc3.set_focus_to(f1);
    press_enter(&mut doc3, &mut h3);
    assert_eq!(h3.clicks_on(def_btn), 2);
    assert!(nav3.take().is_empty());
}
