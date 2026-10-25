//! Integration and regression tests for hybrid glyph dispatch in stroke_text.

#![cfg(target_os = "macos")]

use anyrender::{PaintRef, PaintScene, RenderContext};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, node::TextBrush};
use blitz_paint::bitmap_glyph::{
    BitmapGlyphCache, compute_bitmap_placement_transform, extract_raster_glyph,
};
use blitz_paint::text::{DrawTextContext, stroke_text};
use kurbo::{Affine, Rect, Shape, Stroke};
use parley::LayoutContext;
use parley::style::StyleProperty;
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
        embolden: kurbo::Vec2,
    },
    DrawImage {
        width: u32,
        height: u32,
        transform: Affine,
        blob_id: u64,
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
        _transform: Affine,
        _brush: impl Into<PaintRef<'a>>,
        _brush_transform: Option<Affine>,
        _shape: &impl Shape,
    ) {
    }
    fn fill<'a>(
        &mut self,
        _style: Fill,
        _transform: Affine,
        _brush: impl Into<PaintRef<'a>>,
        _brush_transform: Option<Affine>,
        _shape: &impl Shape,
    ) {
    }
    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        _font: &'a FontData,
        font_size: f32,
        _hint: bool,
        _normalized_coords: &'a [anyrender::NormalizedCoord],
        embolden: kurbo::Vec2,
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
            embolden,
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

fn layout_text(font_bytes: &[u8], text: &str, font_size: f32) -> parley::Layout<TextBrush> {
    let mut font_ctx = blitz_dom::build_single_font_ctx(font_bytes);
    let mut layout_ctx = LayoutContext::new();
    let mut builder = layout_ctx.ranged_builder(&mut font_ctx, text, 1.0, true);
    builder.push_default(StyleProperty::Brush(TextBrush {
        id: NodeId::from_u64(1),
    }));
    builder.push_default(StyleProperty::FontSize(font_size));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout
}

#[test]
fn test_hybrid_dispatch_single_emoji() {
    let Ok(font_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let layout = layout_text(&font_bytes, "😀", 32.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();
    let base_transform = Affine::translate((10.0, 20.0));

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        base_transform,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    assert_eq!(
        scene.events.len(),
        1,
        "Single emoji run must produce exactly 1 DrawImage event and 0 DrawGlyphs events"
    );

    match &scene.events[0] {
        SceneEvent::DrawImage {
            width,
            height,
            transform,
            blob_id,
        } => {
            assert_eq!(*width, 32);
            assert_eq!(*height, 32);
            assert!(*blob_id > 0);
            assert_eq!(transform.as_coeffs()[0], 1.0);
            assert_eq!(transform.as_coeffs()[3], 1.0);
        }
        other => panic!("Expected DrawImage, got {:?}", other),
    }
}

#[test]
fn test_hybrid_dispatch_mixed_run_draw_order_preservation() {
    let Ok(font_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    // ' ' is an outline glyph without raster sbix image (glyph ID 2 in Apple Color Emoji)
    // '😀' is a raster sbix bitmap glyph
    let layout = layout_text(&font_bytes, " 😀 ", 32.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();
    let base_transform = Affine::IDENTITY;

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        base_transform,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    assert_eq!(
        scene.events.len(),
        3,
        "Expected exactly 3 events: DrawGlyphs (leading space) -> DrawImage (emoji) -> DrawGlyphs (trailing space)"
    );

    match &scene.events[0] {
        SceneEvent::DrawGlyphs { glyph_ids, .. } => {
            assert_eq!(
                glyph_ids.len(),
                1,
                "Leading run must contain 1 vector glyph (space)"
            );
        }
        other => panic!("Expected first event to be DrawGlyphs, got {:?}", other),
    }

    match &scene.events[1] {
        SceneEvent::DrawImage { width, height, .. } => {
            assert_eq!(*width, 32);
            assert_eq!(*height, 32);
        }
        other => panic!("Expected second event to be DrawImage, got {:?}", other),
    }

    match &scene.events[2] {
        SceneEvent::DrawGlyphs { glyph_ids, .. } => {
            assert_eq!(
                glyph_ids.len(),
                1,
                "Trailing run must contain 1 vector glyph (space)"
            );
        }
        other => panic!("Expected third event to be DrawGlyphs, got {:?}", other),
    }
}

#[test]
fn test_hybrid_dispatch_multiple_consecutive_emojis() {
    let Ok(font_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let layout = layout_text(&font_bytes, "😀🎉🚀", 32.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();
    let base_transform = Affine::IDENTITY;

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        base_transform,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    assert_eq!(
        scene.events.len(),
        3,
        "Three consecutive emojis must emit 3 DrawImage events and 0 DrawGlyphs events"
    );

    let mut last_x = -1.0f64;
    for (i, event) in scene.events.iter().enumerate() {
        match event {
            SceneEvent::DrawImage {
                width,
                height,
                transform,
                ..
            } => {
                assert_eq!(*width, 32);
                assert_eq!(*height, 32);
                let x = transform.as_coeffs()[4];
                assert!(
                    x > last_x,
                    "Emoji #{} horizontal translation ({}) must be strictly greater than preceding emoji ({})",
                    i,
                    x,
                    last_x
                );
                last_x = x;
            }
            other => panic!("Expected DrawImage at index {}, got {:?}", i, other),
        }
    }
}

#[test]
fn test_vector_only_font_dispatches_only_draw_glyphs() {
    let Ok(font_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let layout = layout_text(&font_bytes, "Hello World", 16.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();
    let base_transform = Affine::IDENTITY;

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        base_transform,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    assert!(!scene.events.is_empty(), "Scene events must not be empty");
    for event in &scene.events {
        match event {
            SceneEvent::DrawGlyphs { .. } => {}
            SceneEvent::DrawImage { .. } => {
                panic!("Vector-only font (Helvetica) must never emit DrawImage events");
            }
        }
    }
}

#[test]
fn test_hybrid_dispatch_blob_id_reuse_across_frames() {
    let Ok(font_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let layout = layout_text(&font_bytes, "😀", 32.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut context = DrawTextContext::default();

    // Frame 1
    let mut scene1 = RecordingScene::default();
    stroke_text(
        &mut scene1,
        layout.lines(),
        &doc,
        Affine::IDENTITY,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );
    let blob_id_frame1 = match &scene1.events[0] {
        SceneEvent::DrawImage { blob_id, .. } => *blob_id,
        _ => panic!("Expected DrawImage on frame 1"),
    };

    // Frame 2
    let mut scene2 = RecordingScene::default();
    stroke_text(
        &mut scene2,
        layout.lines(),
        &doc,
        Affine::IDENTITY,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );
    let blob_id_frame2 = match &scene2.events[0] {
        SceneEvent::DrawImage { blob_id, .. } => *blob_id,
        _ => panic!("Expected DrawImage on frame 2"),
    };

    assert_eq!(
        blob_id_frame1, blob_id_frame2,
        "Cached bitmap glyph must preserve Blob ID across frames for GPU texture atlas reuse"
    );
}

#[test]
fn test_hybrid_dispatch_custom_cache_isolation() {
    let Ok(font_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let layout = layout_text(&font_bytes, "😀", 32.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut context = DrawTextContext::default();
    context.custom_bitmap_glyph_cache = Some(BitmapGlyphCache::new());

    assert_eq!(
        context.with_bitmap_glyph_cache(|c| c.len()),
        0,
        "Custom cache starts empty"
    );

    let mut scene = RecordingScene::default();
    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        Affine::IDENTITY,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    assert_eq!(
        context.with_bitmap_glyph_cache(|c| c.len()),
        1,
        "Custom cache should be populated after stroke_text"
    );
}

#[test]
fn test_hybrid_dispatch_baseline_and_placement_exact_geometry() {
    let Ok(font_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let font_size = 48.0f32;
    let layout = layout_text(&font_bytes, "😀", font_size);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();
    let base_transform = Affine::translate((50.0, 100.0));

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        base_transform,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    assert_eq!(scene.events.len(), 1);

    // Retrieve the positioned glyph from Parley's layout line
    let first_line = layout.lines().next().unwrap();
    let glyph_run = first_line
        .items()
        .find_map(|item| match item {
            parley::PositionedLayoutItem::GlyphRun(r) => Some(r),
            _ => None,
        })
        .unwrap();
    let glyph = glyph_run.positioned_glyphs().next().unwrap();

    let raster_glyph =
        extract_raster_glyph(&font_bytes, 0, glyph.id as u16, font_size.round() as u16)
            .expect("Raster glyph must extract");

    let (expected_placement_transform, _bounds) =
        compute_bitmap_placement_transform(glyph.x, glyph.y, font_size, &raster_glyph);
    let expected_transform = base_transform * expected_placement_transform;

    match &scene.events[0] {
        SceneEvent::DrawImage { transform, .. } => {
            let actual = transform.as_coeffs();
            let expected = expected_transform.as_coeffs();
            let tol = 1e-4;
            for i in 0..6 {
                assert!(
                    (actual[i] - expected[i]).abs() < tol,
                    "Transform coefficient [{}] mismatch: actual {} vs expected {}",
                    i,
                    actual[i],
                    expected[i]
                );
            }
        }
        other => panic!("Expected DrawImage, got {:?}", other),
    }
}

#[test]
fn test_task10_synthetic_embolden_bypassed_for_bitmap_glyphs() {
    let Ok(font_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        return;
    };

    let font_size = 32.0f32;
    // Mixed run with ASCII space and emoji
    let layout = layout_text(&font_bytes, " 😀 ", font_size);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();
    let transform = Affine::IDENTITY;

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        transform,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    // Verify events: DrawGlyphs (vector space) -> DrawImage (bitmap emoji) -> DrawGlyphs (vector space)
    assert_eq!(scene.events.len(), 3);

    // Verify vector glyphs receive the standard embolden vector
    match &scene.events[0] {
        SceneEvent::DrawGlyphs { embolden, .. } => {
            // Embolden vector is passed to vector glyphs
            assert!(embolden.x >= 0.0 && embolden.y >= 0.0);
        }
        other => panic!("Expected DrawGlyphs, got {:?}", other),
    }

    // Verify bitmap glyph was dispatched via DrawImage without any synthetic embolden offset
    let first_line = layout.lines().next().unwrap();
    let glyph_run = first_line
        .items()
        .find_map(|item| match item {
            parley::PositionedLayoutItem::GlyphRun(r) => Some(r),
            _ => None,
        })
        .unwrap();
    let emoji_glyph = glyph_run.positioned_glyphs().nth(1).unwrap();

    let raster_glyph = extract_raster_glyph(
        &font_bytes,
        0,
        emoji_glyph.id as u16,
        font_size.round() as u16,
    )
    .expect("Raster glyph must extract");

    let (expected_placement_transform, _bounds) =
        compute_bitmap_placement_transform(emoji_glyph.x, emoji_glyph.y, font_size, &raster_glyph);

    match &scene.events[1] {
        SceneEvent::DrawImage { transform, .. } => {
            let actual = transform.as_coeffs();
            let expected = expected_placement_transform.as_coeffs();
            let tol = 1e-4;
            for i in 0..6 {
                assert!(
                    (actual[i] - expected[i]).abs() < tol,
                    "Transform coefficient [{}] mismatch: actual {} vs expected {}",
                    i,
                    actual[i],
                    expected[i]
                );
            }
        }
        other => panic!("Expected DrawImage, got {:?}", other),
    }
}
