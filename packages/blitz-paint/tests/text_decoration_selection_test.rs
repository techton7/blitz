//! Integration tests for text decorations, inline backgrounds, and text selection
//! contracts spanning bitmap glyphs (emoji).

#![cfg(target_os = "macos")]

use anyrender::{PaintRef, PaintScene, RenderContext};
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_paint::text::{
    DrawTextContext, draw_inline_backgrounds, draw_text_selection, stroke_text,
};
use kurbo::{Affine, Rect, Shape, Stroke};
use parley::FontContext;
use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
use parley::layout::PositionedLayoutItem;
use peniko::{BlendMode, Color, Fill, FontData, ImageBrushRef, StyleRef};
use std::sync::Arc;

const APPLE_COLOR_EMOJI_PATH: &str = "/System/Library/Fonts/Apple Color Emoji.ttc";
const HELVETICA_PATH: &str = "/System/Library/Fonts/Helvetica.ttc";

#[derive(Debug, Clone, PartialEq)]
enum SceneEvent {
    DrawGlyphs {
        glyph_ids: Vec<u32>,
        font_size: f32,
        transform: Affine,
    },
    DrawImage {
        width: u32,
        height: u32,
        transform: Affine,
        blob_id: u64,
    },
    Stroke {
        transform: Affine,
        bounding_box: Rect,
        width: f64,
    },
    Fill {
        transform: Affine,
        bounding_box: Rect,
    },
}

#[derive(Default)]
struct RecordingScene {
    events: Vec<SceneEvent>,
}

impl RenderContext for RecordingScene {}

impl PaintScene for RecordingScene {
    fn reset(&mut self) {
        self.events.clear();
    }
    fn push_layer(
        &mut self,
        _fill: Fill,
        _blend: impl Into<BlendMode>,
        _alpha: f32,
        _transform: Affine,
        _clip: &impl Shape,
        _filter: Option<Arc<anyrender::Filter>>,
        _backdrop_filter: Option<Arc<anyrender::Filter>>,
    ) {
    }
    fn push_clip_layer(&mut self, _fill: Fill, _transform: Affine, _clip: &impl Shape) {}
    fn pop_layer(&mut self) {}
    fn stroke<'a>(
        &mut self,
        _style: &Stroke,
        transform: Affine,
        _brush: impl Into<PaintRef<'a>>,
        _brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let bbox = shape.bounding_box();
        self.events.push(SceneEvent::Stroke {
            transform,
            bounding_box: bbox,
            width: bbox.width(),
        });
    }
    fn fill<'a>(
        &mut self,
        _style: Fill,
        transform: Affine,
        _brush: impl Into<PaintRef<'a>>,
        _brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let bbox = shape.bounding_box();
        self.events.push(SceneEvent::Fill {
            transform,
            bounding_box: bbox,
        });
    }
    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        _font: &'a FontData,
        font_size: f32,
        _hint: bool,
        _normalized_coords: &'a [anyrender::NormalizedCoord],
        _embolden: kurbo::Vec2,
        _style: impl Into<StyleRef<'a>>,
        _brush: impl Into<PaintRef<'a>>,
        _brush_alpha: f32,
        transform: Affine,
        _glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = anyrender::Glyph> + Clone,
    ) {
        self.events.push(SceneEvent::DrawGlyphs {
            glyph_ids: glyphs.map(|g| g.id).collect(),
            font_size,
            transform,
        });
    }
    fn draw_image(&mut self, image: ImageBrushRef, transform: Affine) {
        self.events.push(SceneEvent::DrawImage {
            width: image.image.width,
            height: image.image.height,
            transform,
            blob_id: image.image.data.id(),
        });
    }
    fn draw_box_shadow(
        &mut self,
        _transform: Affine,
        _rect: Rect,
        _brush: Color,
        _radius: f64,
        _std_dev: f64,
    ) {
    }
}

fn build_dual_font_ctx(helvetica_bytes: &[u8], emoji_bytes: &[u8]) -> FontContext {
    let mut ctx = FontContext {
        source_cache: SourceCache::new_shared(),
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
    };
    let h_reg = ctx
        .collection
        .register_fonts(Blob::new(Arc::new(helvetica_bytes.to_vec()) as _), None);
    let e_reg = ctx
        .collection
        .register_fonts(Blob::new(Arc::new(emoji_bytes.to_vec()) as _), None);

    let mut family_ids: Vec<_> = h_reg.iter().map(|(id, _)| *id).collect();
    family_ids.extend(e_reg.iter().map(|(id, _)| *id));

    for generic in [
        GenericFamily::SansSerif,
        GenericFamily::Serif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
    ] {
        ctx.collection
            .append_generic_families(generic, family_ids.iter().copied());
    }
    ctx
}

fn is_emoji_run(r: &parley::GlyphRun<'_, blitz_dom::node::TextBrush>) -> bool {
    let font_blob = r.run().font().font.data.as_ref();
    let face_index = r.run().font().font.index;
    ttf_parser::Face::parse(font_blob, face_index)
        .ok()
        .map(|f| f.tables().sbix.is_some())
        .unwrap_or(false)
}

fn create_test_doc_config(font_ctx: FontContext) -> DocumentConfig {
    DocumentConfig {
        viewport: Some(blitz_traits::shell::Viewport::new(
            800,
            600,
            1.0,
            blitz_traits::shell::ColorScheme::Light,
        )),
        font_ctx: Some(font_ctx),
        ..Default::default()
    }
}

// =========================================================================
// AC-1: Text decorations span across emoji runs without disruption
// =========================================================================

#[test]
fn test_ac1_underline_spans_across_emoji_run() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };
    let Ok(helvetica_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let font_ctx = build_dual_font_ctx(&helvetica_bytes, &emoji_bytes);

    let html = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body { margin: 0; }
  p { font-size: 24px; text-decoration: underline; margin: 0; white-space: nowrap; }
</style>
</head>
<body>
  <p id="target">Hello 🚀 World</p>
</body>
</html>"#;

    let config = create_test_doc_config(font_ctx);
    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);

    let target_id = doc.get_element_by_id("target").expect("Target node exists");
    let target_node = doc.get_node(target_id).expect("Node exists");
    let inline_layout = target_node
        .element_data()
        .expect("Element data exists")
        .inline_layout_data
        .as_ref()
        .expect("Inline layout exists");

    let layout = &inline_layout.layout;
    let mut scene = RecordingScene::default();
    let mut draw_ctx = DrawTextContext::default();

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        Affine::IDENTITY,
        1.0,
        target_id,
        &mut draw_ctx,
    );

    // Verify hybrid rendering: both vector glyphs (Hello, World) and raster image (🚀) were drawn
    let has_glyphs = scene
        .events
        .iter()
        .any(|e| matches!(e, SceneEvent::DrawGlyphs { .. }));
    let has_image = scene
        .events
        .iter()
        .any(|e| matches!(e, SceneEvent::DrawImage { .. }));
    assert!(has_glyphs, "Must have drawn vector glyphs");
    assert!(
        has_image,
        "Must have drawn emoji bitmap glyph via DrawImage"
    );

    let line = layout.lines().next().expect("First line exists");
    let runs: Vec<_> = line
        .items()
        .filter_map(|item| match item {
            PositionedLayoutItem::GlyphRun(r) => Some(r),
            _ => None,
        })
        .collect();

    assert!(
        runs.len() >= 3,
        "Expected at least 3 runs (text, emoji, text), got {}",
        runs.len()
    );

    let first_run = &runs[0];
    let last_run = &runs[runs.len() - 1];
    let expected_start_x = first_run.offset() as f64;
    let expected_end_x = (last_run.offset() + last_run.advance()) as f64;
    let expected_span_width = expected_end_x - expected_start_x;

    let stroke_event = scene
        .events
        .iter()
        .find_map(|e| match e {
            SceneEvent::Stroke {
                bounding_box,
                width,
                ..
            } => Some((bounding_box, *width)),
            _ => None,
        })
        .expect("Underline stroke event must be generated");

    let (bbox, width) = stroke_event;
    let tol = 1e-3;

    assert!(
        (bbox.x0 - expected_start_x).abs() < tol,
        "Underline start ({}) must match layout start ({})",
        bbox.x0,
        expected_start_x
    );
    assert!(
        (bbox.x1 - expected_end_x).abs() < tol,
        "Underline end ({}) must match layout end ({})",
        bbox.x1,
        expected_end_x
    );
    assert!(
        (width - expected_span_width).abs() < tol,
        "Underline width ({}) must equal full span width ({}) across text and emoji",
        width,
        expected_span_width
    );

    let emoji_run = runs
        .iter()
        .find(|r| is_emoji_run(r))
        .expect("Emoji run must exist");
    let emoji_x0 = emoji_run.offset() as f64;
    let emoji_x1 = emoji_x0 + emoji_run.advance() as f64;

    assert!(bbox.x0 < emoji_x0, "Underline must start before emoji");
    assert!(bbox.x1 > emoji_x1, "Underline must continue after emoji");
}

#[test]
fn test_ac1_line_through_and_overline_span_across_emoji_run() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };
    let Ok(helvetica_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let font_ctx = build_dual_font_ctx(&helvetica_bytes, &emoji_bytes);

    let html = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body { margin: 0; }
  p { font-size: 24px; text-decoration: line-through overline; margin: 0; white-space: nowrap; }
</style>
</head>
<body>
  <p id="target">Text 🚀 More</p>
</body>
</html>"#;

    let config = create_test_doc_config(font_ctx);
    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);

    let target_id = doc.get_element_by_id("target").expect("Target node exists");
    let target_node = doc.get_node(target_id).expect("Node exists");
    let inline_layout = target_node
        .element_data()
        .expect("Element data exists")
        .inline_layout_data
        .as_ref()
        .expect("Inline layout exists");

    let layout = &inline_layout.layout;
    let mut scene = RecordingScene::default();
    let mut draw_ctx = DrawTextContext::default();

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        Affine::IDENTITY,
        1.0,
        target_id,
        &mut draw_ctx,
    );

    let stroke_events: Vec<_> = scene
        .events
        .iter()
        .filter_map(|e| match e {
            SceneEvent::Stroke {
                bounding_box,
                width,
                ..
            } => Some((*bounding_box, *width)),
            _ => None,
        })
        .collect();

    // With line-through and overline, exactly two decoration strokes must be drawn
    assert_eq!(
        stroke_events.len(),
        2,
        "Expected exactly 2 strokes (overline and line-through), got {}",
        stroke_events.len()
    );

    let line = layout.lines().next().expect("First line exists");
    let runs: Vec<_> = line
        .items()
        .filter_map(|item| match item {
            PositionedLayoutItem::GlyphRun(r) => Some(r),
            _ => None,
        })
        .collect();

    let first_run = &runs[0];
    let last_run = &runs[runs.len() - 1];
    let expected_start_x = first_run.offset() as f64;
    let expected_end_x = (last_run.offset() + last_run.advance()) as f64;
    let expected_span_width = expected_end_x - expected_start_x;

    let emoji_run = runs
        .iter()
        .find(|r| is_emoji_run(r))
        .expect("Emoji run must exist");
    let emoji_x0 = emoji_run.offset() as f64;
    let emoji_x1 = emoji_x0 + emoji_run.advance() as f64;

    let tol = 1e-3;
    for (bbox, width) in stroke_events {
        assert!(
            (bbox.x0 - expected_start_x).abs() < tol,
            "Decoration start ({}) must match layout start ({})",
            bbox.x0,
            expected_start_x
        );
        assert!(
            (bbox.x1 - expected_end_x).abs() < tol,
            "Decoration end ({}) must match layout end ({})",
            bbox.x1,
            expected_end_x
        );
        assert!(
            (width - expected_span_width).abs() < tol,
            "Decoration width ({}) must cover full span ({})",
            width,
            expected_span_width
        );
        assert!(bbox.x0 < emoji_x0, "Decoration must start before emoji");
        assert!(bbox.x1 > emoji_x1, "Decoration must continue after emoji");
    }
}

// =========================================================================
// AC-2: Inline background fills cover bounds of emoji runs
// =========================================================================

#[test]
fn test_ac2_inline_background_covers_emoji_bounds() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };
    let Ok(helvetica_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let font_ctx = build_dual_font_ctx(&helvetica_bytes, &emoji_bytes);

    let html = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body { margin: 0; }
  p { font-size: 24px; margin: 0; white-space: nowrap; }
  span.emoji-bg { background-color: rgb(255, 200, 0); }
</style>
</head>
<body>
  <p id="root">Prefix <span id="emoji-span" class="emoji-bg">🚀</span> Suffix</p>
</body>
</html>"#;

    let config = create_test_doc_config(font_ctx);
    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);

    let root_id = doc.get_element_by_id("root").expect("Root node exists");
    let root_node = doc.get_node(root_id).expect("Root node exists");
    let inline_layout = root_node
        .element_data()
        .expect("Element data exists")
        .inline_layout_data
        .as_ref()
        .expect("Inline layout exists");

    let layout = &inline_layout.layout;
    let mut scene = RecordingScene::default();

    draw_inline_backgrounds(&mut scene, layout.lines(), &doc, Affine::IDENTITY, root_id);

    let fill_events: Vec<_> = scene
        .events
        .iter()
        .filter_map(|e| match e {
            SceneEvent::Fill { bounding_box, .. } => Some(*bounding_box),
            _ => None,
        })
        .collect();

    // Exactly one background fill rectangle corresponding to the highlighted emoji span
    assert_eq!(
        fill_events.len(),
        1,
        "Expected exactly 1 inline background fill, got {}",
        fill_events.len()
    );

    let fill_rect = fill_events[0];
    assert!(fill_rect.width() > 0.0 && fill_rect.width().is_finite());
    assert!(fill_rect.height() > 0.0 && fill_rect.height().is_finite());

    // Locate the emoji glyph run
    let line = layout.lines().next().expect("First line exists");
    let emoji_run = line
        .items()
        .find_map(|item| match item {
            PositionedLayoutItem::GlyphRun(r) if is_emoji_run(&r) => Some(r),
            _ => None,
        })
        .expect("Emoji glyph run must exist in layout");

    let metrics = emoji_run.run().font_metrics();
    let expected_x0 = emoji_run.offset() as f64;
    let expected_w = emoji_run.advance() as f64;
    let baseline = emoji_run.baseline() as f64;
    let expected_y0 = baseline - metrics.ascent as f64;
    let expected_y1 = baseline + metrics.descent as f64;
    let expected_h = (metrics.ascent + metrics.descent) as f64;

    let tol = 1e-3;
    assert!(
        (fill_rect.x0 - expected_x0).abs() < tol,
        "Background fill x0 ({}) must match emoji offset ({})",
        fill_rect.x0,
        expected_x0
    );
    assert!(
        (fill_rect.width() - expected_w).abs() < tol,
        "Background fill width ({}) must match emoji advance ({})",
        fill_rect.width(),
        expected_w
    );
    assert!(
        (fill_rect.y0 - expected_y0).abs() < tol,
        "Background fill y0 ({}) must match emoji baseline - ascent ({})",
        fill_rect.y0,
        expected_y0
    );
    assert!(
        (fill_rect.y1 - expected_y1).abs() < tol,
        "Background fill y1 ({}) must match emoji baseline + descent ({})",
        fill_rect.y1,
        expected_y1
    );
    assert!(
        (fill_rect.height() - expected_h).abs() < tol,
        "Background fill height ({}) must equal ascent + descent ({})",
        fill_rect.height(),
        expected_h
    );
}

// =========================================================================
// AC-3: Text selection highlights cover emoji cluster bounds
// =========================================================================

#[test]
fn test_ac3_text_selection_covers_emoji_cluster_bounds() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };
    let Ok(helvetica_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let font_ctx = build_dual_font_ctx(&helvetica_bytes, &emoji_bytes);

    let text_content = "Hello 🚀 World";
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
<style>
  body {{ margin: 0; }}
  p {{ font-size: 24px; margin: 0; white-space: nowrap; }}
</style>
</head>
<body>
  <p id="target">{}</p>
</body>
</html>"#,
        text_content
    );

    let config = create_test_doc_config(font_ctx);
    let mut doc = HtmlDocument::from_html(&html, config);
    doc.resolve(0.0);

    let target_id = doc.get_element_by_id("target").expect("Target node exists");
    let target_node = doc.get_node(target_id).expect("Node exists");
    let inline_layout = target_node
        .element_data()
        .expect("Element data exists")
        .inline_layout_data
        .as_ref()
        .expect("Inline layout exists");

    let layout = &inline_layout.layout;

    // Determine byte range of the emoji
    let emoji_char = '🚀';
    let emoji_byte_start = text_content.find(emoji_char).expect("Emoji found in text");
    let emoji_byte_end = emoji_byte_start + emoji_char.len_utf8();

    let mut scene = RecordingScene::default();
    draw_text_selection(
        &mut scene,
        layout,
        Affine::IDENTITY,
        emoji_byte_start,
        emoji_byte_end,
    );

    let fill_events: Vec<_> = scene
        .events
        .iter()
        .filter_map(|e| match e {
            SceneEvent::Fill { bounding_box, .. } => Some(*bounding_box),
            _ => None,
        })
        .collect();

    assert_eq!(
        fill_events.len(),
        1,
        "Selection over single emoji must produce exactly 1 selection highlight rect"
    );

    let selection_rect = fill_events[0];
    assert!(selection_rect.width() > 0.0 && selection_rect.width().is_finite());
    assert!(selection_rect.height() > 0.0 && selection_rect.height().is_finite());

    // Compare with the emoji glyph cluster geometry in the layout
    let line = layout.lines().next().expect("First line exists");
    let emoji_run = line
        .items()
        .find_map(|item| match item {
            PositionedLayoutItem::GlyphRun(r) if is_emoji_run(&r) => Some(r),
            _ => None,
        })
        .expect("Emoji run must exist");

    let expected_x0 = emoji_run.offset() as f64;
    let expected_w = emoji_run.advance() as f64;
    let expected_x1 = expected_x0 + expected_w;

    let tol = 1e-3;
    assert!(
        (selection_rect.x0 - expected_x0).abs() < tol,
        "Selection start ({}) must match emoji cluster start ({})",
        selection_rect.x0,
        expected_x0
    );
    assert!(
        (selection_rect.x1 - expected_x1).abs() < tol,
        "Selection end ({}) must match emoji cluster end ({})",
        selection_rect.x1,
        expected_x1
    );
    assert!(
        (selection_rect.width() - expected_w).abs() < tol,
        "Selection width ({}) must equal emoji cluster advance ({})",
        selection_rect.width(),
        expected_w
    );

    // Also verify selection spanning the entire string across text and emoji
    let mut full_scene = RecordingScene::default();
    draw_text_selection(
        &mut full_scene,
        layout,
        Affine::IDENTITY,
        0,
        text_content.len(),
    );

    let full_fill_events: Vec<_> = full_scene
        .events
        .iter()
        .filter_map(|e| match e {
            SceneEvent::Fill { bounding_box, .. } => Some(*bounding_box),
            _ => None,
        })
        .collect();

    assert_eq!(
        full_fill_events.len(),
        1,
        "Full line selection produces 1 rect"
    );
    let full_rect = full_fill_events[0];
    assert!(
        full_rect.x0 <= selection_rect.x0,
        "Full selection must start before or at emoji"
    );
    assert!(
        full_rect.x1 >= selection_rect.x1,
        "Full selection must end after or at emoji"
    );
}

#[test]
fn test_ac3_multiple_consecutive_emojis_selection() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };
    let Ok(helvetica_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let font_ctx = build_dual_font_ctx(&helvetica_bytes, &emoji_bytes);

    let text_content = "😀🎉🚀";
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
<style>
  body {{ margin: 0; }}
  p {{ font-size: 32px; margin: 0; white-space: nowrap; }}
</style>
</head>
<body>
  <p id="target">{}</p>
</body>
</html>"#,
        text_content
    );

    let config = create_test_doc_config(font_ctx);
    let mut doc = HtmlDocument::from_html(&html, config);
    doc.resolve(0.0);

    let target_id = doc.get_element_by_id("target").expect("Target node exists");
    let target_node = doc.get_node(target_id).expect("Node exists");
    let inline_layout = target_node
        .element_data()
        .expect("Element data exists")
        .inline_layout_data
        .as_ref()
        .expect("Inline layout exists");

    let layout = &inline_layout.layout;

    // Select the middle emoji '🎉'
    let middle_char = '🎉';
    let middle_start = text_content.find(middle_char).expect("Middle emoji found");
    let middle_end = middle_start + middle_char.len_utf8();

    let mut scene = RecordingScene::default();
    draw_text_selection(
        &mut scene,
        layout,
        Affine::IDENTITY,
        middle_start,
        middle_end,
    );

    let fill_events: Vec<_> = scene
        .events
        .iter()
        .filter_map(|e| match e {
            SceneEvent::Fill { bounding_box, .. } => Some(*bounding_box),
            _ => None,
        })
        .collect();

    assert_eq!(
        fill_events.len(),
        1,
        "Middle emoji selection must produce 1 highlight rect"
    );
    let sel_rect = fill_events[0];
    assert!(sel_rect.width() > 0.0 && sel_rect.width().is_finite());
    assert!(sel_rect.height() > 0.0 && sel_rect.height().is_finite());
    assert!(
        sel_rect.x0 > 0.0,
        "Middle emoji selection must start after the first emoji"
    );
}
