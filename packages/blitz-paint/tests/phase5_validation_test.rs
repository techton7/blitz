//! Phase 5 Validation Tests for macOS Color Emoji in Blitz.
//!
//! - Task 11 (01M57TSHSX4HNWJPRDAZMDRGJ4): `verify-mixed-text-emoji-baseline-alignment`
//! - Task 12 (01M57TSNV8E42E7MVVPVPB163T): `verify-complex-emoji-sequences`
//! - Task 13 (01M57TST6BWZ8HSP0DNKNRDWEG): `verify-retina-scaling-and-container-clipping`

#![cfg(target_os = "macos")]

use std::sync::Arc;
use anyrender::{PaintRef, PaintScene, RenderContext};
use blitz_dom::{BaseDocument, DocumentConfig, FontContext, NodeId, node::TextBrush};
use blitz_html::HtmlDocument;
use blitz_paint::bitmap_glyph::{
    compute_bitmap_placement_transform, extract_raster_glyph,
};
use blitz_paint::paint_scene;
use blitz_paint::text::{stroke_text, DrawTextContext};
use kurbo::{Affine, Point, Rect, Shape, Stroke};
use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
use parley::style::StyleProperty;
use parley::LayoutContext;
use peniko::{BlendMode, Color, Fill, FontData, ImageBrushRef, StyleRef};

const APPLE_COLOR_EMOJI_PATH: &str = "/System/Library/Fonts/Apple Color Emoji.ttc";
const HELVETICA_PATH: &str = "/System/Library/Fonts/Helvetica.ttc";
const PNG_MAGIC_HEADER: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[derive(Debug, Clone, PartialEq)]
enum SceneEvent {
    PushLayer {
        opacity: f32,
        transform: Affine,
    },
    PushClipLayer {
        transform: Affine,
    },
    PopLayer,
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
        opacity: f32,
        transform: Affine,
        _clip: &impl Shape,
        _filter: Option<Arc<anyrender::Filter>>,
        _backdrop_filter: Option<Arc<anyrender::Filter>>,
    ) {
        self.events.push(SceneEvent::PushLayer { opacity, transform });
    }
    fn push_clip_layer(&mut self, _fill: Fill, transform: Affine, _clip: &impl Shape) {
        self.events.push(SceneEvent::PushClipLayer { transform });
    }
    fn pop_layer(&mut self) {
        self.events.push(SceneEvent::PopLayer);
    }
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

fn layout_text_with_scale(
    font_ctx: &mut FontContext,
    text: &str,
    font_size: f32,
    scale: f32,
) -> parley::Layout<TextBrush> {
    let mut layout_ctx = LayoutContext::new();
    let mut builder = layout_ctx.ranged_builder(font_ctx, text, scale, true);
    builder.push_default(StyleProperty::Brush(TextBrush {
        id: NodeId::from_u64(1),
    }));
    builder.push_default(StyleProperty::FontSize(font_size));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout
}

fn layout_text(font_ctx: &mut FontContext, text: &str, font_size: f32) -> parley::Layout<TextBrush> {
    layout_text_with_scale(font_ctx, text, font_size, 1.0)
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
// Task 11: verify-mixed-text-emoji-baseline-alignment
// =========================================================================

#[test]
fn test_task11_ac1_mixed_text_emoji_shared_baseline_coordinate() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };
    let Ok(helvetica_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let mut font_ctx = build_dual_font_ctx(&helvetica_bytes, &emoji_bytes);
    let font_size = 32.0;
    let layout = layout_text(&mut font_ctx, "Hello 🌍 World", font_size);

    assert_eq!(layout.lines().count(), 1, "Expected single line for 'Hello 🌍 World'");
    let line = layout.lines().next().expect("Line 0 exists");
    let line_baseline = line.metrics().baseline;

    let mut has_latin_glyph = false;
    let mut has_emoji_glyph = false;
    let mut total_glyphs = 0;

    for item in line.items() {
        if let parley::PositionedLayoutItem::GlyphRun(run) = item {
            // Verify run baseline matches the line baseline
            assert_eq!(
                run.baseline(),
                line_baseline,
                "Glyph run baseline must match the line baseline"
            );

            let is_sbix_font = {
                let font_blob = run.run().font().font.data.as_ref();
                let face_index = run.run().font().font.index;
                ttf_parser::Face::parse(font_blob, face_index)
                    .ok()
                    .map(|f| f.tables().sbix.is_some())
                    .unwrap_or(false)
            };

            for glyph in run.positioned_glyphs() {
                total_glyphs += 1;
                // AC-1: Latin glyphs and emoji share the exact same baseline coordinate
                assert_eq!(
                    glyph.y, line_baseline,
                    "Every glyph (Latin and emoji) must share exact same baseline coordinate (glyph.y == line_baseline)"
                );

                if is_sbix_font {
                    has_emoji_glyph = true;
                } else {
                    has_latin_glyph = true;
                }
            }
        }
    }

    assert!(has_latin_glyph, "Layout must contain Latin glyphs");
    assert!(has_emoji_glyph, "Layout must contain emoji glyph");
    assert!(total_glyphs >= 12, "Must contain all characters of 'Hello 🌍 World'");
}

#[test]
fn test_task11_ac2_emoji_placement_transform_baseline_and_descender() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&emoji_bytes, 0).expect("Face 0 should parse");
    let earth_gid = face
        .glyph_index('🌍')
        .expect("Glyph index for '🌍' should exist");

    let test_cases = [
        // (font_size, target_ppem, expected_strike_ppem, glyph_x, glyph_y)
        (32.0, 32, 32, 100.0, 200.0),
        (48.0, 48, 48, 150.0, 300.0),
        (64.0, 64, 64, 50.0, 120.0),
        (24.0, 24, 26, 80.0, 180.0), // Strike 26 scaled to 24px
    ];

    for (font_size, target_ppem, expected_strike, glyph_x, glyph_y) in test_cases {
        let raster_glyph = extract_raster_glyph(&emoji_bytes, 0, earth_gid.0, target_ppem)
            .expect("Raster glyph for '🌍' must exist");
        assert_eq!(
            raster_glyph.strike_ppem, expected_strike,
            "Extracted strike ppem must match expected strike"
        );
        assert!(
            raster_glyph.origin_y < 0.0,
            "origin_y must be negative downwards for baseline descender normalization"
        );

        let strike_scale = font_size / raster_glyph.strike_ppem as f32;
        let (placement_transform, bounds) = compute_bitmap_placement_transform(
            glyph_x,
            glyph_y,
            font_size,
            &raster_glyph,
        );

        // AC-2: Verify computed placement transform places the emoji's baseline precisely on glyph.y
        let local_baseline_y = (raster_glyph.origin_y + raster_glyph.height) as f64;
        let local_baseline_point = Point::new(0.0, local_baseline_y);
        let canvas_baseline_point = placement_transform * local_baseline_point;

        assert!(
            (canvas_baseline_point.y - glyph_y as f64).abs() < 1e-4,
            "Mapped baseline Y ({}) must match glyph.y ({}) at font_size={}",
            canvas_baseline_point.y,
            glyph_y,
            font_size
        );

        // AC-2: Descender (-origin_y * strike_scale) extends below glyph.y
        let expected_descender_bottom = glyph_y as f64 + (-raster_glyph.origin_y * strike_scale) as f64;
        assert!(
            (bounds.y1 - expected_descender_bottom).abs() < 1e-4,
            "Bottom bound ({}) must match glyph_y + (-origin_y * strike_scale) ({})",
            bounds.y1,
            expected_descender_bottom
        );
        assert!(
            bounds.y1 > glyph_y as f64,
            "Emoji descender must extend strictly below glyph.y"
        );

        // AC-2: Ascent extends above glyph.y
        let expected_ascent_top = glyph_y as f64 - ((raster_glyph.origin_y + raster_glyph.height) * strike_scale) as f64;
        assert!(
            (bounds.y0 - expected_ascent_top).abs() < 1e-4,
            "Top bound ({}) must match glyph_y - ((origin_y + height) * strike_scale) ({})",
            bounds.y0,
            expected_ascent_top
        );
        assert!(
            bounds.y0 < glyph_y as f64,
            "Emoji ascent must extend strictly above glyph.y"
        );

        // Bounding box dimensions match scaled strike dimensions
        let expected_width = (raster_glyph.width * strike_scale) as f64;
        let expected_height = (raster_glyph.height * strike_scale) as f64;
        assert!((bounds.width() - expected_width).abs() < 1e-4);
        assert!((bounds.height() - expected_height).abs() < 1e-4);
    }
}

// =========================================================================
// Task 12: verify-complex-emoji-sequences
// =========================================================================

#[test]
fn test_task12_complex_zwj_sequences() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let mut emoji_ctx = blitz_dom::build_single_font_ctx(&emoji_bytes);
    let zwj_sequences = [
        ("technologist", "🧑‍💻"),
        ("rainbow_flag", "🏳️‍🌈"),
        ("family_mwgb", "👨‍👩‍👧‍👦"),
    ];

    for (name, sequence) in zwj_sequences {
        let layout = layout_text(&mut emoji_ctx, sequence, 32.0);
        let doc = BaseDocument::new(DocumentConfig::default());
        let mut scene = RecordingScene::default();
        let mut context = DrawTextContext::default();

        let mut glyph_count = 0;
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(run) = item {
                    for glyph in run.positioned_glyphs() {
                        glyph_count += 1;
                        let raster = extract_raster_glyph(&emoji_bytes, 0, glyph.id as u16, 32);
                        let raster_glyph = raster.unwrap_or_else(|| {
                            panic!("Sequence '{name}' ({sequence}) produced glyph ID {} without raster glyph", glyph.id)
                        });

                        assert_eq!(
                            &raster_glyph.data[..8],
                            &PNG_MAGIC_HEADER,
                            "Raster glyph data for '{name}' must contain valid PNG magic header"
                        );
                        assert!(raster_glyph.width > 0.0);
                        assert!(raster_glyph.height > 0.0);
                        assert_eq!(raster_glyph.strike_ppem, 32);
                    }
                }
            }
        }

        assert!(
            glyph_count >= 1,
            "Parley must shape sequence '{name}' into at least 1 glyph run"
        );

        stroke_text(
            &mut scene,
            layout.lines(),
            &doc,
            Affine::IDENTITY,
            1.0,
            NodeId::from_u64(0),
            &mut context,
        );

        let image_events: Vec<_> = scene
            .events
            .iter()
            .filter_map(|e| match e {
                SceneEvent::DrawImage { width, height, blob_id, transform } => {
                    Some((*width, *height, *blob_id, *transform))
                }
                _ => None,
            })
            .collect();

        assert_eq!(
            image_events.len(),
            glyph_count,
            "Sequence '{name}' must dispatch exactly {glyph_count} DrawImage events"
        );

        for (w, h, blob_id, transform) in image_events {
            assert_eq!(w, 32);
            assert_eq!(h, 32);
            assert!(blob_id > 0);
            assert_eq!(transform.as_coeffs()[0], 1.0);
            assert_eq!(transform.as_coeffs()[3], 1.0);
        }
    }
}

#[test]
fn test_task12_skin_tone_modifiers() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let mut emoji_ctx = blitz_dom::build_single_font_ctx(&emoji_bytes);
    let skin_tone_sequences = [
        ("thumbs_up_medium", "👍🏽"),
        ("waving_hand_dark", "👋🏿"),
    ];

    for (name, sequence) in skin_tone_sequences {
        let layout = layout_text(&mut emoji_ctx, sequence, 32.0);
        let doc = BaseDocument::new(DocumentConfig::default());
        let mut scene = RecordingScene::default();
        let mut context = DrawTextContext::default();

        let mut glyph_count = 0;
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(run) = item {
                    for glyph in run.positioned_glyphs() {
                        glyph_count += 1;
                        let raster = extract_raster_glyph(&emoji_bytes, 0, glyph.id as u16, 32);
                        let raster_glyph = raster.unwrap_or_else(|| {
                            panic!("Skin tone sequence '{name}' produced glyph ID {} without raster glyph", glyph.id)
                        });

                        assert_eq!(&raster_glyph.data[..8], &PNG_MAGIC_HEADER);
                        assert!(raster_glyph.width > 0.0);
                        assert!(raster_glyph.height > 0.0);
                    }
                }
            }
        }

        assert!(glyph_count >= 1);

        stroke_text(
            &mut scene,
            layout.lines(),
            &doc,
            Affine::IDENTITY,
            1.0,
            NodeId::from_u64(0),
            &mut context,
        );

        let image_count = scene
            .events
            .iter()
            .filter(|e| matches!(e, SceneEvent::DrawImage { .. }))
            .count();

        assert_eq!(
            image_count, glyph_count,
            "Skin tone sequence '{name}' must render all glyphs via DrawImage"
        );
    }
}

#[test]
fn test_task12_variation_selector() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let mut emoji_ctx = blitz_dom::build_single_font_ctx(&emoji_bytes);
    // \u{263A}\u{FE0F} is smiling face with emoji presentation variation selector
    let sequence = "\u{263A}\u{FE0F}";
    let layout = layout_text(&mut emoji_ctx, sequence, 32.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();

    let mut glyph_count = 0;
    for line in layout.lines() {
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(run) = item {
                for glyph in run.positioned_glyphs() {
                    glyph_count += 1;
                    let raster = extract_raster_glyph(&emoji_bytes, 0, glyph.id as u16, 32);
                    let raster_glyph = raster.expect("Variation selector sequence must extract raster glyph");
                    assert_eq!(&raster_glyph.data[..8], &PNG_MAGIC_HEADER);
                }
            }
        }
    }

    assert!(glyph_count >= 1);

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        Affine::IDENTITY,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    let image_count = scene
        .events
        .iter()
        .filter(|e| matches!(e, SceneEvent::DrawImage { .. }))
        .count();

    assert_eq!(
        image_count, glyph_count,
        "Variation selector sequence must render via DrawImage"
    );
}

#[test]
fn test_task12_mixed_complex_sequences_in_line() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };
    let Ok(helvetica_bytes) = std::fs::read(HELVETICA_PATH) else {
        eprintln!("Skipping test: Helvetica not found at {HELVETICA_PATH}");
        return;
    };

    let mut font_ctx = build_dual_font_ctx(&helvetica_bytes, &emoji_bytes);
    let text = "Dev: 🧑‍💻 👍🏽 🏳️‍🌈 Done";
    let layout = layout_text(&mut font_ctx, text, 32.0);
    let doc = BaseDocument::new(DocumentConfig::default());
    let mut scene = RecordingScene::default();
    let mut context = DrawTextContext::default();

    stroke_text(
        &mut scene,
        layout.lines(),
        &doc,
        Affine::IDENTITY,
        1.0,
        NodeId::from_u64(0),
        &mut context,
    );

    let vector_count = scene
        .events
        .iter()
        .filter(|e| matches!(e, SceneEvent::DrawGlyphs { .. }))
        .count();
    let image_count = scene
        .events
        .iter()
        .filter(|e| matches!(e, SceneEvent::DrawImage { .. }))
        .count();

    assert!(vector_count >= 2, "Must contain at least 2 DrawGlyphs runs (for 'Dev: ' and ' Done')");
    assert_eq!(image_count, 3, "Must contain exactly 3 DrawImage events (for 🧑‍💻, 👍🏽, 🏳️‍🌈)");
}

// =========================================================================
// Task 13: verify-retina-scaling-and-container-clipping
// =========================================================================

#[test]
fn test_task13_ac1_retina_scaling_strike_selection_and_transforms() {
    let Ok(emoji_bytes) = std::fs::read(APPLE_COLOR_EMOJI_PATH) else {
        eprintln!("Skipping test: Apple Color Emoji not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&emoji_bytes, 0).expect("Face 0 should parse");
    let smile_gid = face
        .glyph_index('😀')
        .expect("Glyph index for '😀' should exist");

    let css_font_size = 16.0f32;
    let scales = [1.0f32, 2.0f32, 3.0f32];

    for &scale in &scales {
        let scaled_font_size = css_font_size * scale;
        let target_ppem = scaled_font_size.round().max(1.0) as u16;

        let raster_glyph = extract_raster_glyph(&emoji_bytes, 0, smile_gid.0, target_ppem)
            .unwrap_or_else(|| panic!("Raster glyph must exist at target_ppem={target_ppem}"));

        // Strike selection adapts to target_ppem
        match scale as i32 {
            1 => {
                // target_ppem = 16 -> strike 20 selected
                assert_eq!(target_ppem, 16);
                assert_eq!(raster_glyph.strike_ppem, 20);
            }
            2 => {
                // target_ppem = 32 -> strike 32 selected
                assert_eq!(target_ppem, 32);
                assert_eq!(raster_glyph.strike_ppem, 32);
            }
            3 => {
                // target_ppem = 48 -> strike 48 selected
                assert_eq!(target_ppem, 48);
                assert_eq!(raster_glyph.strike_ppem, 48);
            }
            _ => unreachable!(),
        }

        let strike_scale = scaled_font_size / raster_glyph.strike_ppem as f32;
        let glyph_x = 10.0 * scale;
        let glyph_y = 20.0 * scale;

        let (transform, bounds) = compute_bitmap_placement_transform(
            glyph_x,
            glyph_y,
            scaled_font_size,
            &raster_glyph,
        );

        // Dimensions match dest_w = width * strike_scale and dest_h = height * strike_scale
        let expected_dest_w = (raster_glyph.width * strike_scale) as f64;
        let expected_dest_h = (raster_glyph.height * strike_scale) as f64;
        assert!(
            (bounds.width() - expected_dest_w).abs() < 1e-4,
            "bounds.width() ({}) must match expected dest_w ({}) at scale={}",
            bounds.width(),
            expected_dest_w,
            scale
        );
        assert!(
            (bounds.height() - expected_dest_h).abs() < 1e-4,
            "bounds.height() ({}) must match expected dest_h ({}) at scale={}",
            bounds.height(),
            expected_dest_h,
            scale
        );

        // Destination dimensions scale proportionally with retina scale
        let expected_css_size = css_font_size as f64;
        let expected_canvas_size = expected_css_size * scale as f64;
        assert!(
            (bounds.width() - expected_canvas_size).abs() < 1e-4,
            "dest_w ({}) must equal css_font_size * scale ({})",
            bounds.width(),
            expected_canvas_size
        );

        // Verify placement transform scale coefficient matches strike_scale
        let coeffs = transform.as_coeffs();
        assert!(
            (coeffs[0] - strike_scale as f64).abs() < 1e-4,
            "Transform X-scale ({}) must match strike_scale ({})",
            coeffs[0],
            strike_scale
        );
        assert!(
            (coeffs[3] - strike_scale as f64).abs() < 1e-4,
            "Transform Y-scale ({}) must match strike_scale ({})",
            coeffs[3],
            strike_scale
        );
    }
}

#[test]
fn test_task13_ac2_container_clipping_nests_draw_image() {
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
  body { margin: 0; padding: 0; }
  .clipped-container {
    width: 100px;
    height: 100px;
    overflow: hidden;
  }
</style>
</head>
<body>
  <div class="clipped-container">
    <span>Hello 🌍 World</span>
  </div>
</body>
</html>"#;

    let config = create_test_doc_config(font_ctx);
    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);

    let mut scene = RecordingScene::default();
    paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);

    // AC-2: Container clipping nests DrawImage between push_clip_layer and pop_layer
    let mut clip_depth: i32 = 0;
    let mut image_clip_depths = Vec::new();
    let mut push_clip_count = 0;
    let mut pop_count = 0;

    for event in &scene.events {
        match event {
            SceneEvent::PushClipLayer { .. } => {
                clip_depth += 1;
                push_clip_count += 1;
            }
            SceneEvent::PopLayer => {
                clip_depth -= 1;
                pop_count += 1;
            }
            SceneEvent::DrawImage { width, height, blob_id, .. } => {
                image_clip_depths.push((clip_depth, *width, *height, *blob_id));
            }
            _ => {}
        }
    }

    assert!(push_clip_count >= 1, "Must have pushed at least 1 clip layer");
    assert!(pop_count >= 1, "Must have popped at least 1 layer");
    assert_eq!(
        image_clip_depths.len(),
        1,
        "Must have dispatched exactly 1 DrawImage event for 🌍"
    );

    let (image_depth, width, height, blob_id) = image_clip_depths[0];
    assert!(
        image_depth >= 1,
        "DrawImage must be strictly nested inside a clip layer (depth={image_depth} >= 1)"
    );
    assert!(width > 0);
    assert!(height > 0);
    assert!(blob_id > 0);
}

#[test]
fn test_task13_ac2_unclipped_container_does_not_push_clip_layer() {
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
  body { margin: 0; padding: 0; }
  .unclipped-container {
    width: 100px;
    height: 100px;
    overflow: visible;
  }
</style>
</head>
<body>
  <div class="unclipped-container">
    <span>Hello 🌍 World</span>
  </div>
</body>
</html>"#;

    let config = create_test_doc_config(font_ctx);
    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);

    let mut scene = RecordingScene::default();
    paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);

    let push_clip_count = scene
        .events
        .iter()
        .filter(|e| matches!(e, SceneEvent::PushClipLayer { .. }))
        .count();
    let has_image = scene
        .events
        .iter()
        .any(|e| matches!(e, SceneEvent::DrawImage { .. }));

    assert_eq!(
        push_clip_count, 0,
        "Unclipped container (overflow: visible) must not push clip layers"
    );
    assert!(has_image, "Must still draw the emoji image");
}
