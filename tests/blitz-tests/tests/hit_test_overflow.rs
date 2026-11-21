use blitz_dom::ScrollBehavior;
use blitz_test_harness::{Harness, HarnessOptions};

fn hit_id(harness: &Harness, x: f32, y: f32) -> String {
    let node_id = harness.hit_node(x, y);
    let doc = harness.base();
    doc.get_node(node_id)
        .and_then(|n| n.data.downcast_element())
        .and_then(|el| el.id.as_ref().map(|id| id.to_string()))
        .unwrap_or_default()
}

#[test]
fn overflow_auto_scroll_hidden_clip_children_outside_scrollport() {
    for overflow in ["auto", "scroll", "hidden"] {
        let harness = Harness::from_html_with(
            &format!(
                r#"<html><body id="body" style="margin:0; position:relative">
                    <div id="underlay" style="position:absolute; top:200px; left:0; width:200px; height:100px"></div>
                    <div id="box" style="width:200px; height:180px; overflow:{overflow}">
                        <div id="tall" style="width:200px; height:700px"></div>
                    </div>
                </body></html>"#
            ),
            HarnessOptions {
                width: 400,
                height: 400,
                ..Default::default()
            },
        );

        // Inside the 180px box hits the tall child.
        assert_eq!(hit_id(&harness, 50.0, 100.0), "tall", "overflow:{overflow}");
        // In empty space directly below the 180px box (y=250): must hit #underlay, never #tall.
        assert_eq!(
            hit_id(&harness, 50.0, 250.0),
            "underlay",
            "overflow:{overflow}"
        );
    }
}

#[test]
fn scrolled_overflow_auto_clips_top_and_bottom_and_hits_scrolled_child() {
    let mut harness = Harness::from_html_with(
        r#"<html><body id="body" style="margin:0; position:relative; display:flow-root; height:400px">
            <div id="underlay-top" style="position:absolute; top:0; left:0; width:200px; height:40px"></div>
            <div id="underlay-bottom" style="position:absolute; top:240px; left:0; width:200px; height:100px"></div>
            <div id="box" style="margin-top:50px; width:200px; height:180px; overflow:auto">
                <div id="first" style="width:200px; height:200px"></div>
                <div id="second" style="width:200px; height:300px"></div>
            </div>
        </body></html>"#,
        HarnessOptions {
            width: 400,
            height: 400,
            ..Default::default()
        },
    );

    let box_id = harness.node("#box");
    harness
        .base_mut()
        .scroll_to(box_id, 0.0, 250.0, ScrollBehavior::Instant);
    harness.pump();

    // Box spans y=[50..230]. Scrolled 250px down, #second (y=[200..500] in content) fills the box.
    assert_eq!(hit_id(&harness, 50.0, 100.0), "second");
    // Above the box (y=20) hits #underlay-top, never #first scrolled above the scrollport.
    assert_eq!(hit_id(&harness, 50.0, 20.0), "underlay-top");
    // Below the box (y=260) hits #underlay-bottom, never #second clipped below the scrollport.
    assert_eq!(hit_id(&harness, 50.0, 260.0), "underlay-bottom");
}

#[test]
fn overflow_clip_and_margin_and_contain_paint() {
    let harness = Harness::from_html_with(
        r#"<html><body id="body" style="margin:0; display:flow-root; height:500px">
            <div id="clip-box" style="width:100px; height:100px; overflow:clip; overflow-clip-margin:20px">
                <div id="clip-child" style="width:100px; height:300px"></div>
            </div>
            <div id="contain-box" style="margin-top:50px; width:100px; height:100px; contain:paint; overflow:clip; overflow-clip-margin:20px">
                <div id="contain-child" style="width:100px; height:300px"></div>
            </div>
        </body></html>"#,
        HarnessOptions {
            width: 400,
            height: 500,
            ..Default::default()
        },
    );

    // #clip-box is [0..100], overflow-clip-margin: 20px extends clip edge to y=120.
    assert_eq!(hit_id(&harness, 50.0, 110.0), "clip-child");
    assert_eq!(hit_id(&harness, 50.0, 130.0), "body");

    // #contain-box starts at y=150 (100px + 50px margin) -> [150..250].
    // contain:paint ignores overflow-clip-margin, so y=260 is clipped and hits body.
    assert_eq!(hit_id(&harness, 50.0, 200.0), "contain-child");
    assert_eq!(hit_id(&harness, 50.0, 260.0), "body");
}

#[test]
fn z_index_abspos_escapes_clips_below_cb_but_clipped_by_cb_and_above() {
    let harness = Harness::from_html_with(
        r#"<html><body id="body" style="margin:0">
            <!-- #sc is CB for #escaped (because #non-cb-clipper is position:static) -->
            <div id="sc" style="position:relative; z-index:0; width:200px; height:250px">
                <div id="non-cb-clipper" style="width:100px; height:100px; overflow:hidden">
                    <div id="escaped" style="position:absolute; top:140px; left:10px; width:60px; height:60px; z-index:2"></div>
                </div>
            </div>
            <!-- #cb-clipper IS CB for #trapped, hoisted into #outer-sc above #cb-clipper -->
            <div id="outer-sc" style="position:relative; z-index:0; width:200px; height:250px">
                <div id="cb-clipper" style="position:relative; width:100px; height:100px; overflow:hidden">
                    <div id="trapped" style="position:absolute; top:140px; left:10px; width:60px; height:60px; z-index:2"></div>
                </div>
            </div>
        </body></html>"#,
        HarnessOptions {
            width: 400,
            height: 600,
            ..Default::default()
        },
    );

    // #escaped escapes #non-cb-clipper (below CB #sc) -> hit at (30, 160).
    assert_eq!(hit_id(&harness, 30.0, 160.0), "escaped");

    // #outer-sc starts at y=250; #cb-clipper is [250..350].
    // #trapped is at y = 250 + 140 = 390, clipped by its CB #cb-clipper -> hits #outer-sc.
    assert_eq!(hit_id(&harness, 30.0, 410.0), "outer-sc");
}

#[test]
fn padded_inline_root_hit_result_uses_content_box_coordinates() {
    let harness = Harness::from_html_with(
        r#"<html><body style="margin:0">
            <div id="pad" style="padding:20px; width:200px; height:40px; overflow:hidden; font-size:16px; line-height:20px">hello</div>
        </body></html>"#,
        HarnessOptions {
            width: 400,
            height: 200,
            ..Default::default()
        },
    );

    // Hit inside the top-left padding of #pad at (5, 5): content-box relative coords are (-15, -15).
    let pad_id = harness.node("#pad");
    let hit = harness.hit(5.0, 5.0).expect("expected hit on #pad");
    assert_eq!(hit.node_id, pad_id);
    assert!(!hit.is_text);
    assert_eq!(hit.x, -15.0);
    assert_eq!(hit.y, -15.0);
}
