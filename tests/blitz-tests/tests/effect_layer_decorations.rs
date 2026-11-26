use anyrender::recording::RenderCommand;
use anyrender::{Scene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use std::sync::Arc;

const CANVAS: u32 = 120;

fn doc(html: &str, scale: f32) -> HtmlDocument {
    let mut d = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(CANVAS, CANVAS, scale, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    d.resolve(0.0);
    d
}

fn sample(html: &str, scale: f32, x: f32, y: f32) -> [u8; 3] {
    let s = (CANVAS as f32 * scale).round() as u32;
    let mut d = doc(html, scale);
    let b = render_to_buffer::<VelloCpuImageRenderer, _>(
        |sc| paint_scene(sc, &mut d, scale as f64, s, s, 0, 0),
        s,
        s,
    );
    let i = ((y * scale).round() as usize * s as usize + (x * scale).round() as usize) * 4;
    [b[i], b[i + 1], b[i + 2]]
}

fn sample_box(style: &str, scale: f32, x: f32, y: f32) -> [u8; 3] {
    sample(
        &format!(
            "<html><body style='margin:0;background:#fff'><div style='margin:30px;width:60px;height:60px;{style}'></div></body></html>"
        ),
        scale,
        x,
        y,
    )
}

fn scene(html: &str) -> Scene {
    let mut sc = Scene::new();
    paint_scene(&mut sc, &mut doc(html, 1.0), 1.0, CANVAS, CANVAS, 0, 0);
    sc
}

#[test]
fn opacity_fades_outset_box_shadow_and_outline() {
    for s in [0.5, 1.0, 2.0] {
        let o1 = sample_box("opacity:1.0;outline:10px solid #f00", s, 25.0, 60.0);
        let oh = sample_box("opacity:0.5;outline:10px solid #f00", s, 25.0, 60.0);
        assert!(o1[0] >= 253 && o1[1] <= 2 && oh[0] >= 253 && (oh[1] as i32 - 128).abs() <= 2);

        let s1 = sample_box("opacity:1.0;box-shadow:0 0 0 10px #00f", s, 25.0, 60.0);
        let sh = sample_box("opacity:0.5;box-shadow:0 0 0 10px #00f", s, 25.0, 60.0);
        assert!(s1[2] >= 253 && s1[0] <= 2 && sh[2] >= 253 && (sh[0] as i32 - 128).abs() <= 2);

        let blur = |op| {
            255.0
                - sample_box(
                    &format!("opacity:{op};box-shadow:0 0 8px #00f"),
                    s,
                    28.0,
                    60.0,
                )[0] as f32
        };
        assert!((blur("0.5") - blur("1.0") * 0.5).abs() <= 3.0 && blur("1.0") > 20.0);

        let t = sample_box(
            "background:transparent;opacity:0.5;box-shadow:0 0 0 12px #00f",
            s,
            60.0,
            60.0,
        );
        assert_eq!(t, [255, 255, 255]);
    }
}

#[test]
fn group_compositing_and_overflow_clip_preserve_paint_order() {
    for s in [0.5, 1.0, 2.0] {
        let g = sample_box(
            "opacity:0.5;box-shadow:0 0 0 12px #f00,0 0 0 12px #00f",
            s,
            22.0,
            60.0,
        );
        assert!(g[0] >= 254 && (g[1] as i32 - 128).abs() <= 1 && (g[2] as i32 - 128).abs() <= 1);

        let html = "<html><body style='margin:0;background:#fff'>\
            <div style='margin:30px;width:60px;height:60px;overflow:hidden;opacity:0.5;outline:6px solid #0f0;box-shadow:0 0 0 12px #00f;background:#fff'>\
                <div style='width:200px;height:200px;background:#ff0'></div></div></body></html>";
        let (outline, shadow, outside) = (
            sample(html, s, 27.0, 60.0),
            sample(html, s, 21.0, 60.0),
            sample(html, s, 10.0, 60.0),
        );
        assert!(outline[1] >= 253 && (outline[0] as i32 - 128).abs() <= 2);
        assert!(shadow[2] >= 253 && (shadow[0] as i32 - 128).abs() <= 2);
        assert_eq!(outside, [255, 255, 255]);
    }
}

#[test]
fn filter_and_backdrop_filter_layer_bounds() {
    let f_scene = scene(
        "<html><body style='margin:0;background:#000'><div style='margin:30px;width:60px;height:60px;filter:invert(1);outline:6px solid #f00;box-shadow:0 0 0 12px #f00'></div></body></html>",
    );
    let (idx, f_layer) = f_scene
        .commands
        .iter()
        .enumerate()
        .find_map(|(i, c)| match c {
            RenderCommand::PushLayer(l) if l.filter.is_some() => Some((i, l)),
            _ => None,
        })
        .expect("filter layer");
    assert_eq!(f_layer.clip.to_svg(), "M-12,-12 L72,-12 L72,72 L-12,72 Z");

    let pop_idx = f_scene.commands[idx..]
        .iter()
        .position(|c| matches!(c, RenderCommand::PopLayer))
        .map(|i| idx + i)
        .unwrap();
    let inner = &f_scene.commands[idx + 1..pop_idx];
    assert!(
        inner
            .iter()
            .any(|c| matches!(c, RenderCommand::BoxShadow(_)))
    );
    assert!(inner.iter().any(|c| matches!(c, RenderCommand::Fill(_))));

    let bd_scene = scene(
        "<html><body style='margin:0;background:#fff'><div style='margin:20px;width:60px;height:60px;backdrop-filter:blur(4px);opacity:0.5;box-shadow:0 0 0 12px rgb(0,0,255)'></div></body></html>",
    );
    let layers: Vec<_> = bd_scene
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::PushLayer(l) => Some(l),
            _ => None,
        })
        .collect();
    let bd = layers.iter().find(|l| l.backdrop_filter.is_some()).unwrap();
    let op = layers
        .iter()
        .find(|l| l.backdrop_filter.is_none() && (l.alpha - 0.5).abs() < 1e-4)
        .unwrap();
    assert_eq!(bd.clip.to_svg(), "M0,0 L60,0 L60,60 L0,60 Z");
    assert_eq!(op.clip.to_svg(), "M-12,-12 L72,-12 L72,72 L-12,72 Z");
}

#[test]
fn visible_overflow_protruding_arrow_in_opacity_layer() {
    // Tooltip/popover pattern: container at (30, 30) 60x40 (bottom edge y=70) with a
    // protruding rotated arrow (12x12 at left:24px; bottom:-6px; rotate:45deg), whose
    // center sits at (60, 70) and extends down to y ~= 78.5 outside the border box.
    for s in [0.5, 1.0, 2.0] {
        let visible_html = "<html><body style='margin:0;background:#fff'>\
            <div style='position:relative;margin:30px;width:60px;height:40px;overflow:visible;opacity:0.5;background:#00f'>\
                <div style='position:absolute;left:24px;bottom:-6px;width:12px;height:12px;rotate:45deg;background:#00f'></div>\
            </div></body></html>";
        // Sample inside the protruding arrow tip below border_box (y=74 is 4px below y=70).
        let visible_tip = sample(visible_html, s, 60.0, 74.0);
        assert!(
            visible_tip[2] >= 253
                && (visible_tip[0] as i32 - 128).abs() <= 2
                && (visible_tip[1] as i32 - 128).abs() <= 2,
            "scale {s}: visible protruding arrow must paint outside border box at 50% alpha, got {visible_tip:?}"
        );

        let hidden_html = "<html><body style='margin:0;background:#fff'>\
            <div style='position:relative;margin:30px;width:60px;height:40px;overflow:hidden;opacity:0.5;background:#00f'>\
                <div style='position:absolute;left:24px;bottom:-6px;width:12px;height:12px;rotate:45deg;background:#00f'></div>\
            </div></body></html>";
        let hidden_tip = sample(hidden_html, s, 60.0, 74.0);
        assert_eq!(
            hidden_tip,
            [255, 255, 255],
            "scale {s}: overflow:hidden must still clip the protruding arrow"
        );
    }

    // With backdrop-filter + opacity + overflow:visible + protruding arrow:
    // backdrop-filter stays strictly clipped to border_box (60x40), while the opacity
    // layer expands past y=40 to paint the protruding arrow at 50% alpha.
    let backdrop_html = "<html><body style='margin:0;background:#fff'>\
        <div style='position:relative;margin:30px;width:60px;height:40px;overflow:visible;opacity:0.5;backdrop-filter:blur(4px);background:#00f'>\
            <div style='position:absolute;left:24px;bottom:-6px;width:12px;height:12px;rotate:45deg;background:#00f'></div>\
        </div></body></html>";
    let bd_scene = scene(backdrop_html);
    let layers: Vec<_> = bd_scene
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::PushLayer(l) => Some(l),
            _ => None,
        })
        .collect();
    let bd = layers
        .iter()
        .find(|l| l.backdrop_filter.is_some())
        .expect("backdrop_filter layer");
    let op = layers
        .iter()
        .find(|l| l.backdrop_filter.is_none() && (l.alpha - 0.5).abs() < 1e-4)
        .expect("split opacity layer");
    assert_eq!(
        bd.clip.to_svg(),
        "M0,0 L60,0 L60,40 L0,40 Z",
        "backdrop-filter layer must stay clipped to border_box"
    );
    let op_svg = op.clip.to_svg();
    let max_y = op_svg
        .split_whitespace()
        .filter_map(|tok| tok.split_once(',').and_then(|(_, y)| y.parse::<f64>().ok()))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        max_y > 46.0,
        "opacity layer clip must extend past border_box to include rotated arrow, got {op_svg}"
    );
    let tip = sample(backdrop_html, 1.0, 60.0, 74.0);
    assert!(
        tip[2] >= 253 && (tip[0] as i32 - 128).abs() <= 2 && (tip[1] as i32 - 128).abs() <= 2,
        "protruding arrow with backdrop-filter must still paint outside border_box at 50% alpha, got {tip:?}"
    );
}
