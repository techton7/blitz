//! Integration tests for default keyboard scrolling (`Arrow*`, `PageUp`/`PageDown`,
//! `Home`/`End`, `Space`/`Shift+Space`).

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use blitz_dom::{Document, DocumentConfig, EventDriver, EventHandler, NodeId};
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::{
    events::{BlitzKeyEvent, DomEvent, DomEventData, EventState, KeyState, UiEvent},
    shell::{ColorScheme, ShellProvider, Viewport},
};
use keyboard_types::{Code, Key, Location, Modifiers, NamedKey};

#[derive(Default)]
struct RecordingShell {
    redraws: AtomicUsize,
}

impl ShellProvider for RecordingShell {
    fn request_redraw(&self) {
        self.redraws.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct CancellingHandler {
    cancel_keydown_for: Option<Key>,
}

impl EventHandler for &mut CancellingHandler {
    fn handle_event(
        &mut self,
        _chain: &[NodeId],
        event: &mut DomEvent,
        _doc: &mut dyn Document,
        event_state: &mut EventState,
    ) {
        if let DomEventData::KeyDown(ke) = &event.data
            && self.cancel_keydown_for.as_ref() == Some(&ke.key)
        {
            event_state.prevent_default();
        }
    }
}

fn make_doc(html: &str) -> (HtmlDocument, Arc<RecordingShell>) {
    let shell = Arc::new(RecordingShell::default());
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            shell_provider: Some(shell.clone() as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    (doc, shell)
}

fn send_key(
    doc: &mut HtmlDocument,
    handler: &mut CancellingHandler,
    key: Key,
    code: Code,
    modifiers: Modifiers,
) {
    let text = match &key {
        Key::Character(s) => Some(s.as_str().into()),
        _ => None,
    };
    let mut driver = EventDriver::new(doc, handler);
    driver.handle_ui_event(UiEvent::KeyDown(BlitzKeyEvent {
        key,
        code,
        modifiers,
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text,
    }));
}

fn sel(doc: &HtmlDocument, selector: &str) -> NodeId {
    doc.query_selector(selector).unwrap().expect(selector)
}

#[test]
fn viewport_scrolls_on_arrows_page_home_end_and_space() {
    let html = r#"
        <html><body style="margin:0">
            <div style="width:1200px;height:2000px"></div>
        </body></html>
    "#;
    let (mut doc, shell) = make_doc(html);
    let mut handler = CancellingHandler::default();

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowDown),
        Code::ArrowDown,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 40.0);
    assert!(shell.redraws.load(Ordering::SeqCst) > 0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowUp),
        Code::ArrowUp,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 0.0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowRight),
        Code::ArrowRight,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().x, 40.0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowLeft),
        Code::ArrowLeft,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().x, 0.0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::PageDown),
        Code::PageDown,
        Modifiers::empty(),
    );
    assert!((doc.viewport_scroll().y - 262.5).abs() < 0.01);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::PageUp),
        Code::PageUp,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 0.0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Character(" ".into()),
        Code::Space,
        Modifiers::empty(),
    );
    assert!((doc.viewport_scroll().y - 262.5).abs() < 0.01);

    send_key(
        &mut doc,
        &mut handler,
        Key::Character(" ".into()),
        Code::Space,
        Modifiers::SHIFT,
    );
    assert_eq!(doc.viewport_scroll().y, 0.0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::End),
        Code::End,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 1700.0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::Home),
        Code::Home,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 0.0);
}

#[test]
fn focused_scroll_container_scrolls_and_chains_to_viewport() {
    let html = r#"
        <html><body style="margin:0">
            <div id="scroller" tabindex="0" style="width:200px;height:100px;overflow:auto">
                <div style="height:220px"></div>
            </div>
            <div style="height:1000px"></div>
        </body></html>
    "#;
    let (mut doc, _shell) = make_doc(html);
    let scroller = sel(&doc, "#scroller");
    doc.set_focus_to(scroller);
    let mut handler = CancellingHandler::default();

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowDown),
        Code::ArrowDown,
        Modifiers::empty(),
    );
    assert_eq!(doc.get_node(scroller).unwrap().scroll_offset().y, 40.0);
    assert_eq!(doc.viewport_scroll().y, 0.0);

    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::End),
        Code::End,
        Modifiers::empty(),
    );
    assert_eq!(doc.get_node(scroller).unwrap().scroll_offset().y, 120.0);
    assert_eq!(doc.viewport_scroll().y, 0.0);

    // Once the focused container hits its bottom edge, ArrowDown chains to the viewport.
    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowDown),
        Code::ArrowDown,
        Modifiers::empty(),
    );
    assert_eq!(doc.get_node(scroller).unwrap().scroll_offset().y, 120.0);
    assert_eq!(doc.viewport_scroll().y, 40.0);
}

#[test]
fn prevent_default_button_space_and_text_input_do_not_scroll() {
    let html = r#"
        <html><body style="margin:0">
            <div id="slider" tabindex="0" role="slider" style="width:100px;height:20px"></div>
            <button id="btn">Click</button>
            <input id="txt" type="text" value="hello" />
            <div style="height:2000px"></div>
        </body></html>
    "#;
    let (mut doc, _shell) = make_doc(html);

    let slider = sel(&doc, "#slider");
    doc.set_focus_to(slider);
    let mut handler = CancellingHandler {
        cancel_keydown_for: Some(Key::Named(NamedKey::ArrowDown)),
    };
    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowDown),
        Code::ArrowDown,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 0.0);

    let btn = sel(&doc, "#btn");
    doc.set_focus_to(btn);
    let mut handler = CancellingHandler::default();
    send_key(
        &mut doc,
        &mut handler,
        Key::Character(" ".into()),
        Code::Space,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 0.0);

    let txt = sel(&doc, "#txt");
    doc.set_focus_to(txt);
    send_key(
        &mut doc,
        &mut handler,
        Key::Named(NamedKey::ArrowDown),
        Code::ArrowDown,
        Modifiers::empty(),
    );
    send_key(
        &mut doc,
        &mut handler,
        Key::Character(" ".into()),
        Code::Space,
        Modifiers::empty(),
    );
    assert_eq!(doc.viewport_scroll().y, 0.0);
}
