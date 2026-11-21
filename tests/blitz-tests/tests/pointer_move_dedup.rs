//! Scroll-triggered hover refreshes must not be followed by duplicate mouse
//! pointer-move compatibility events.

use blitz_test_harness::{Harness, HarnessOptions, pointer_event};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, BlitzWheelEvent, MouseEventButton,
    MouseEventButtons, Point, PointerCoords, UiEvent,
};
use keyboard_types::Modifiers;

fn mouse_event(client_x: f32, client_y: f32, page_x: f32, page_y: f32) -> BlitzPointerEvent {
    let mut event = pointer_event(
        BlitzPointerId::Mouse,
        page_x,
        page_y,
        MouseEventButton::Main,
        MouseEventButtons::empty(),
        Modifiers::default(),
    );
    event.coords.client_x = client_x;
    event.coords.client_y = client_y;
    event.coords.screen_x = client_x;
    event.coords.screen_y = client_y;
    event
}

fn wheel_event(coords: PointerCoords) -> UiEvent {
    UiEvent::Wheel(BlitzWheelEvent {
        delta: BlitzWheelDelta::Pixels(0.0, -100.0),
        phase: Default::default(),
        source: Default::default(),
        coords,
        buttons: MouseEventButtons::empty(),
        mods: Modifiers::default(),
        element: Point::default(),
    })
}

fn assert_no_mouse_moves(events: &[String]) {
    assert!(
        !events.iter().any(|name| name == "pointermove"),
        "{events:?}"
    );
    assert!(!events.iter().any(|name| name == "mousemove"), "{events:?}");
}

#[test]
fn scroll_hover_refresh_deduplicates_same_position_mouse_move() {
    let mut harness = Harness::from_html_with(
        r#"<html><body style="margin:0">
            <div id="scroller" style="width:100px;height:100px;overflow-y:scroll">
                <div id="item1" style="height:100px"></div>
                <div id="item2" style="height:100px"></div>
            </div>
        </body></html>"#,
        HarnessOptions {
            width: 200,
            height: 200,
            ..Default::default()
        },
    );
    let scroller = harness.node("#scroller");
    let item1 = harness.node("#item1");
    let item2 = harness.node("#item2");
    let initial = mouse_event(50.0, 50.0, 50.0, 50.0);

    harness.dispatch_recorded([UiEvent::PointerMove(initial.clone())]);
    harness.pump();
    assert_eq!(harness.hovered(), Some(item1));
    assert!(harness.base().get_node(item1).unwrap().is_hovered());

    harness.dispatch(wheel_event(initial.coords));
    harness.pump();

    assert_eq!(
        harness.base().get_node(scroller).unwrap().scroll_offset().y,
        100.0
    );
    assert_eq!(harness.hovered(), Some(item2));
    assert!(!harness.base().get_node(item1).unwrap().is_hovered());
    assert!(harness.base().get_node(item2).unwrap().is_hovered());

    let events = harness.dispatch_recorded([UiEvent::PointerMove(initial)]);
    assert_no_mouse_moves(&events);
}

#[test]
fn viewport_scroll_deduplicates_unchanged_client_position() {
    let mut harness = Harness::from_html_with(
        r#"<html><body style="margin:0">
            <div id="item1" style="height:100px"></div>
            <div id="item2" style="height:100px"></div>
        </body></html>"#,
        HarnessOptions {
            width: 100,
            height: 100,
            ..Default::default()
        },
    );
    let item1 = harness.node("#item1");
    let item2 = harness.node("#item2");
    let initial = mouse_event(50.0, 50.0, 50.0, 50.0);

    harness.dispatch_recorded([UiEvent::PointerMove(initial.clone())]);
    harness.pump();
    assert_eq!(harness.hovered(), Some(item1));

    harness.dispatch(wheel_event(initial.coords));
    harness.pump();

    let scroll_y = harness.base().viewport_scroll().y;
    assert_eq!(scroll_y, 100.0);
    assert_eq!(harness.hovered(), Some(item2));

    let after_scroll = mouse_event(50.0, 50.0, 50.0, 50.0 + scroll_y as f32);
    assert_eq!(after_scroll.client_x(), initial.client_x());
    assert_eq!(after_scroll.client_y(), initial.client_y());
    assert_ne!(after_scroll.page_y(), initial.page_y());

    let events = harness.dispatch_recorded([UiEvent::PointerMove(after_scroll)]);
    assert_no_mouse_moves(&events);
    assert_eq!(harness.hovered(), Some(item2));
}

#[test]
fn real_move_and_reentry_after_leave_still_dispatch() {
    let mut harness = Harness::from_html_with(
        r#"<html><body style="margin:0">
            <div id="item" style="width:100px;height:100px"></div>
        </body></html>"#,
        HarnessOptions {
            width: 200,
            height: 200,
            ..Default::default()
        },
    );
    let item = harness.node("#item");
    let original = mouse_event(50.0, 50.0, 50.0, 50.0);

    let initial = harness.dispatch_recorded([UiEvent::PointerMove(original.clone())]);
    assert_eq!(
        initial.iter().filter(|name| *name == "pointermove").count(),
        1
    );
    assert_eq!(
        initial.iter().filter(|name| *name == "mousemove").count(),
        1
    );
    assert_eq!(harness.hovered(), Some(item));

    let moved = mouse_event(51.0, 50.0, 51.0, 50.0);
    let moved_events = harness.dispatch_recorded([UiEvent::PointerMove(moved.clone())]);
    assert_eq!(
        moved_events
            .iter()
            .filter(|name| *name == "pointermove")
            .count(),
        1
    );
    assert_eq!(
        moved_events
            .iter()
            .filter(|name| *name == "mousemove")
            .count(),
        1
    );

    let leave = mouse_event(51.0, 50.0, 51.0, 50.0);
    harness.dispatch_recorded([UiEvent::PointerLeave(leave)]);
    assert_eq!(harness.hovered(), None);

    let reentered = harness.dispatch_recorded([UiEvent::PointerMove(moved)]);
    assert_eq!(
        reentered
            .iter()
            .filter(|name| *name == "pointermove")
            .count(),
        1
    );
    assert_eq!(
        reentered.iter().filter(|name| *name == "mousemove").count(),
        1
    );
    assert_eq!(harness.hovered(), Some(item));
}
