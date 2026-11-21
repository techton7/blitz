use blitz_test_harness::Harness;
use blitz_traits::events::BlitzWheelPhase::{Ended, Moved, Started};
use blitz_traits::events::BlitzWheelSource::{Finger, Momentum, Wheel};

fn nested_scroller_page(inner_style: &str) -> Harness {
    Harness::from_html(&format!(
        r#"<html><body style="margin:0; height:3000px">
            <div id="inner" style="width:120px; height:100px; overflow:auto; {inner_style}">
                <div style="width:120px; height:150px"></div>
            </div>
        </body></html>"#
    ))
}

#[test]
fn trackpad_gesture_and_momentum_stay_latched_to_inner_scroller() {
    let mut harness = nested_scroller_page("");
    let inner = harness.node("#inner");

    // Inner has max scroll = 50px (150px content in 100px box).
    // Hit boundary mid-gesture and continue through finger lift + momentum.
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -30.0, Started, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Moved, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -50.0, Ended, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Started, Momentum);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Moved, Momentum);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Momentum);

    assert_eq!(harness.base().get_node(inner).unwrap().scroll_offset().y, 50.0);
    assert_eq!(harness.base().viewport_scroll().y, 0.0);
}

#[test]
fn fresh_gesture_after_momentum_end_chains_unless_contained() {
    // overscroll-behavior: auto -> fresh gesture at boundary chains to outer document
    let mut harness = nested_scroller_page("");
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Started, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -50.0, Started, Momentum);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Momentum);
    assert_eq!(harness.base().viewport_scroll().y, 0.0);

    harness.wheel_gesture_at(50.0, 50.0, 0.0, -40.0, Started, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Finger);
    assert_eq!(harness.base().viewport_scroll().y, 40.0);

    // overscroll-behavior: contain -> fresh gesture at boundary still stops chaining
    let mut contained = nested_scroller_page("overscroll-behavior: contain");
    contained.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Started, Finger);
    contained.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Finger);
    contained.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Started, Momentum);
    contained.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Momentum);
    contained.wheel_gesture_at(50.0, 50.0, 0.0, -40.0, Started, Finger);
    contained.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Finger);
    assert_eq!(contained.base().viewport_scroll().y, 0.0);
}

#[test]
fn zero_delta_started_then_moved_latches_inner_scroller() {
    let mut harness = nested_scroller_page("");
    let inner = harness.node("#inner");

    harness.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Started, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -30.0, Moved, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Moved, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Finger);

    assert_eq!(harness.base().get_node(inner).unwrap().scroll_offset().y, 50.0);
    assert_eq!(harness.base().viewport_scroll().y, 0.0);
}

#[test]
fn discrete_wheel_ticks_stay_latched_until_pointer_moves() {
    let mut harness = nested_scroller_page("");
    let inner = harness.node("#inner");

    // Consecutive stationary ticks within 500ms stay clamped to inner scroller.
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -30.0, Moved, Wheel);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Moved, Wheel);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, -100.0, Moved, Wheel);
    assert_eq!(harness.base().get_node(inner).unwrap().scroll_offset().y, 50.0);
    assert_eq!(harness.base().viewport_scroll().y, 0.0);

    // Moving the pointer >1px clears the latch and starts a fresh chain to outer document.
    harness.wheel_gesture_at(52.0, 50.0, 0.0, -40.0, Moved, Wheel);
    assert_eq!(harness.base().viewport_scroll().y, 40.0);
}

#[test]
fn latched_text_input_clamps_mid_gesture_and_chains_on_fresh_gesture() {
    let lines = (0..40)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut harness = Harness::from_html(&format!(
        r#"<html><body style="margin:0; height:3000px">
            <textarea id="ta" style="width:140px; height:50px; border:0; padding:0">{lines}</textarea>
        </body></html>"#
    ));
    let ta = harness.node("#ta");
    let (cx, cy) = harness.center_of("#ta");

    let (max_scroll, content_h) = {
        let doc = harness.base();
        let node = doc.get_node(ta).unwrap();
        let layout = node.final_layout();
        let input = node.element_data().unwrap().text_input_data().unwrap();
        (
            input.max_scroll_offset(layout.content_box_width(), layout.content_box_height()),
            layout.content_box_height(),
        )
    };
    assert!(
        max_scroll > 0.0 && content_h > 0.0,
        "textarea must overflow vertically for latch test"
    );

    harness.wheel_gesture_at(cx, cy, 0.0, -10.0, Started, Finger);
    harness.wheel_gesture_at(cx, cy, 0.0, -5000.0, Moved, Finger);
    harness.wheel_gesture_at(cx, cy, 0.0, -500.0, Ended, Finger);
    harness.wheel_gesture_at(cx, cy, 0.0, -500.0, Started, Momentum);
    harness.wheel_gesture_at(cx, cy, 0.0, -500.0, Moved, Momentum);
    harness.wheel_gesture_at(cx, cy, 0.0, 0.0, Ended, Momentum);

    let scroll_after = harness
        .base()
        .get_node(ta)
        .unwrap()
        .element_data()
        .unwrap()
        .text_input_data()
        .unwrap()
        .scroll_offset;
    assert!((scroll_after - max_scroll).abs() < 0.01);
    assert_eq!(harness.base().viewport_scroll().y, 0.0);

    // Fresh gesture at boundary chains to outer document.
    harness.wheel_gesture_at(cx, cy, 0.0, -35.0, Started, Finger);
    harness.wheel_gesture_at(cx, cy, 0.0, 0.0, Ended, Finger);
    assert_eq!(harness.base().viewport_scroll().y, 35.0);
}

#[test]
fn removing_latched_node_clears_latch_cleanly() {
    let mut harness = nested_scroller_page("");
    let inner = harness.node("#inner");

    harness.wheel_gesture_at(50.0, 50.0, 0.0, -10.0, Started, Finger);
    assert_eq!(harness.base().get_node(inner).unwrap().scroll_offset().y, 10.0);

    {
        let mut doc = harness.base_mut();
        doc.mutate().remove_and_drop_node(inner);
    }
    harness.pump();

    harness.wheel_gesture_at(50.0, 50.0, 0.0, -40.0, Moved, Finger);
    harness.wheel_gesture_at(50.0, 50.0, 0.0, 0.0, Ended, Finger);
    assert_eq!(harness.base().viewport_scroll().y, 40.0);
}
