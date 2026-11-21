//! Regression tests for `outline-style: auto` rendering and effect-layer
//! `outline_bounds` reconciliation with `outline-offset`.

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use std::sync::Arc;

const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

fn pixel_at(html: &str, x: u32, y: u32) -> [u8; 3] {
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(160, 100, 1.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| paint_scene(scene, &mut doc, 1.0, 160, 100, 0, 0),
        160,
        100,
    );
    let idx = ((y * 160 + x) * 4) as usize;
    [buffer[idx], buffer[idx + 1], buffer[idx + 2]]
}

#[test]
fn outline_auto_renders_with_default_medium_width_and_explicit_color() {
    let html = "<html><body style='margin:0;background:#fff'>\
        <div style='position:absolute;left:20px;top:20px;width:100px;height:60px;\
                    outline:auto;outline-color:#0000ff'></div></body></html>";
    // Default `outline-width: medium` is 3px outside the border box (x 17..20).
    assert_eq!(pixel_at(html, 18, 50), BLUE);
    assert_eq!(pixel_at(html, 21, 50), WHITE);
}

#[test]
fn outline_auto_with_opacity_and_offset_expands_effect_bounds() {
    let html = "<html><body style='margin:0;background:#fff'>\
        <div style='position:absolute;left:20px;top:20px;width:100px;height:60px;\
                    opacity:0.5;outline:2px auto #0000ff;outline-offset:4px'></div></body></html>";
    // Outer ring sits at x 14..16; 0.5 opacity over white gives ~[128, 128, 255].
    let px = pixel_at(html, 15, 50);
    assert!(
        px[2] >= 253 && (px[0] as i32 - 128).abs() <= 2,
        "got {px:?}"
    );
    assert_eq!(pixel_at(html, 18, 50), WHITE);
}
