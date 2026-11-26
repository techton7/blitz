//! Text decorations propagate from block/flex containers into their in-flow block children
//! (including anonymous blocks wrapping direct text children), while stopping at atomic
//! inlines, floats, and out-of-flow boxes (`DioxusLabs/blitz#1660`).

use anyrender::recording::RenderCommand;
use anyrender::{Paint, Scene};
use blitz_dom::{DocumentConfig, FontContext};
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use std::sync::Arc;

fn paint_html(html: &str) -> Scene {
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(200, 100, 1.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            font_ctx: Some(FontContext::new()),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    let mut scene = Scene::new();
    paint_scene(&mut scene, doc.as_mut(), 1.0, 200, 100, 0, 0);
    scene
}

fn has_red_fill(scene: &Scene) -> bool {
    scene.commands.iter().any(|cmd| match cmd {
        RenderCommand::Fill(fill) => match &fill.brush {
            Paint::Solid(color) => {
                let [r, g, b, a] = color.components;
                r > 0.9 && g < 0.1 && b < 0.1 && a > 0.9
            }
            _ => false,
        },
        _ => false,
    })
}

#[test]
fn flex_container_propagates_line_through_to_anonymous_block_text() {
    let scene = paint_html(
        r#"<html><body style="margin:0;background:#fff">
            <div data-calendar-day style="display:flex;font-size:20px;color:#000;text-decoration:line-through #ff0000">15</div>
        </body></html>"#,
    );
    assert!(
        has_red_fill(&scene),
        "display:flex container must propagate text-decoration: line-through to direct text child"
    );
}

#[test]
fn inline_flex_keeps_own_line_through_for_anonymous_block_text() {
    let scene = paint_html(
        r#"<html><body style="margin:0;background:#fff">
            <div style="display:inline-flex;font-size:20px;color:#000;text-decoration:line-through #ff0000">15</div>
        </body></html>"#,
    );
    assert!(
        has_red_fill(&scene),
        "display:inline-flex container must draw its own text-decoration: line-through on direct text child"
    );
}

#[test]
fn atomic_inline_stops_ancestor_line_through_propagation() {
    let scene = paint_html(
        r#"<html><body style="margin:0;background:#fff">
            <div style="font-size:20px;color:#000;text-decoration:line-through #ff0000">
                <span style="display:inline-block">15</span>
            </div>
        </body></html>"#,
    );
    assert!(
        !has_red_fill(&scene),
        "ancestor text-decoration must not propagate into atomic inline-block"
    );
}
