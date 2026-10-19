//! Contract and integration tests for bitmap glyph extraction and normalization.

#![cfg(target_os = "macos")]

use blitz_paint::bitmap_glyph::{
    compute_bitmap_placement_transform, extract_raster_glyph, BitmapCacheKey, BitmapGlyphCache,
};
use kurbo::Point;
use peniko::{ImageAlphaType, ImageFormat};

const APPLE_COLOR_EMOJI_PATH: &str = "/System/Library/Fonts/Apple Color Emoji.ttc";
const PNG_MAGIC_HEADER: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

fn load_apple_color_emoji() -> Option<Vec<u8>> {
    std::fs::read(APPLE_COLOR_EMOJI_PATH).ok()
}

#[test]
fn test_apple_color_emoji_contract_representative_glyphs() {
    let Some(font_bytes) = load_apple_color_emoji() else {
        eprintln!("Skipping Apple Color Emoji contract test: file not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Face 0 should parse");

    // Outline / non-bitmap glyph IDs (such as glyph 2 and glyph 10) must return None
    assert!(
        extract_raster_glyph(&font_bytes, 0, 2, 32).is_none(),
        "Glyph ID 2 in Apple Color Emoji is an outline glyph without raster images and must return None"
    );
    assert!(
        extract_raster_glyph(&font_bytes, 0, 10, 32).is_none(),
        "Glyph ID 10 in Apple Color Emoji is an outline glyph without raster images and must return None"
    );

    // Representative emoji characters and their glyph IDs
    let test_chars = ['😀', '🎉', '🚀', '🔥', '👍'];
    for ch in test_chars {
        let gid = face
            .glyph_index(ch)
            .unwrap_or_else(|| panic!("Glyph index for character '{ch}' should exist"));
        let glyph = extract_raster_glyph(&font_bytes, 0, gid.0, 32)
            .unwrap_or_else(|| panic!("Raster glyph for character '{ch}' (glyph ID {gid:?}) should be extracted"));

        assert_eq!(
            &glyph.data[..8],
            &PNG_MAGIC_HEADER,
            "Extracted emoji glyph for '{ch}' must contain PNG magic header"
        );
        assert!(glyph.width > 0.0, "Glyph width must be positive");
        assert!(glyph.height > 0.0, "Glyph height must be positive");
        assert_eq!(glyph.strike_ppem, 32, "Requested ppem 32 should yield strike ppem 32");
    }

    // Direct bitmap glyph index (first available sbix glyph in Apple Color Emoji)
    let first_sbix_gid = 112;
    let glyph = extract_raster_glyph(&font_bytes, 0, first_sbix_gid, 32)
        .expect("Glyph 112 must be extracted as a valid raster glyph");
    assert_eq!(&glyph.data[..8], &PNG_MAGIC_HEADER);
    assert!(glyph.width > 0.0);
    assert!(glyph.height > 0.0);
}

#[test]
fn test_apple_color_emoji_contract_origin_y_descender_normalization() {
    let Some(font_bytes) = load_apple_color_emoji() else {
        eprintln!("Skipping Apple Color Emoji contract test: file not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Face 0 should parse");
    let smile_gid = face
        .glyph_index('😀')
        .expect("Glyph ID for '😀' should exist");

    // Verify raw image condition: sbix tables in Apple Color Emoji store raw y == 0
    let raw_image = face
        .glyph_raster_image(smile_gid, 32)
        .expect("Raw raster image should exist for '😀' at ppem 32");
    assert_eq!(
        raw_image.y, 0,
        "Apple Color Emoji raw sbix image must have image.y == 0"
    );

    // Verify that extract_raster_glyph normalizes origin_y negative downwards (descender)
    let glyph = extract_raster_glyph(&font_bytes, 0, smile_gid.0, 32)
        .expect("Normalized raster glyph should be extracted");

    assert!(
        glyph.origin_y < 0.0,
        "Normalized origin_y must be negative downwards for baseline descender alignment (got {})",
        glyph.origin_y
    );

    // Units per em is 800 in Apple Color Emoji, default descender fallback is -100 units:
    // origin_y = (-100.0 * 32.0) / 800.0 = -4.0
    assert_eq!(
        glyph.origin_y, -4.0,
        "origin_y must equal exact scaled descender value"
    );
    assert_eq!(glyph.origin_x, 0.0);
}

#[test]
fn test_apple_color_emoji_contract_discrete_strike_sizes() {
    let Some(font_bytes) = load_apple_color_emoji() else {
        eprintln!("Skipping Apple Color Emoji contract test: file not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Face 0 should parse");
    let smile_gid = face
        .glyph_index('😀')
        .expect("Glyph ID for '😀' should exist");

    // Discrete strike sizes in Apple Color Emoji: 20, 32, 40, 64, 96, 160
    let target_sizes: &[u16] = &[20, 32, 40, 64, 96, 160];

    for &target_ppem in target_sizes {
        let glyph = extract_raster_glyph(&font_bytes, 0, smile_gid.0, target_ppem)
            .unwrap_or_else(|| panic!("Failed to extract glyph for target_ppem {target_ppem}"));

        assert_eq!(
            glyph.strike_ppem, target_ppem,
            "Target ppem {target_ppem} should select strike with exact matching ppem"
        );
        assert_eq!(glyph.width, target_ppem as f32);
        assert_eq!(glyph.height, target_ppem as f32);
        assert_eq!(&glyph.data[..8], &PNG_MAGIC_HEADER);

        let expected_origin_y = (-100.0 * target_ppem as f32) / (face.units_per_em() as f32);
        assert_eq!(
            glyph.origin_y, expected_origin_y,
            "origin_y for ppem {target_ppem} should equal {expected_origin_y}"
        );
    }
}

#[test]
fn test_apple_color_emoji_contract_intermediate_strike_selection() {
    let Some(font_bytes) = load_apple_color_emoji() else {
        eprintln!("Skipping Apple Color Emoji contract test: file not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Face 0 should parse");
    let smile_gid = face
        .glyph_index('😀')
        .expect("Glyph ID for '😀' should exist");

    // Intermediate requested sizes should select the closest strike >= target_ppem
    // Available strikes include: 20, 26, 32, 40, 48, 52, 64, 96, 160
    let test_cases = [
        (24, 26),
        (28, 32),
        (35, 40),
        (50, 52),
        (70, 96),
        (120, 160),
        (200, 160), // Exceeds all strikes, picks max strike
    ];

    for (requested, expected_strike) in test_cases {
        let glyph = extract_raster_glyph(&font_bytes, 0, smile_gid.0, requested)
            .unwrap_or_else(|| panic!("Failed to extract glyph for requested ppem {requested}"));

        assert_eq!(
            glyph.strike_ppem, expected_strike,
            "Requested ppem {requested} should select strike {expected_strike}"
        );
        assert_eq!(&glyph.data[..8], &PNG_MAGIC_HEADER);
    }
}

#[test]
fn test_apple_color_emoji_pipeline_integration() {
    let Some(font_bytes) = load_apple_color_emoji() else {
        eprintln!("Skipping Apple Color Emoji contract test: file not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Face 0 should parse");
    let mut cache = BitmapGlyphCache::new();

    let test_cases = [
        ('😀', 13.333f32),
        ('🎉', 16.5f32),
        ('🚀', 27.8125f32),
        ('🔥', 32.0f32),
        ('👍', 64.0f32),
        ('😀', 104.25f32),
    ];

    let glyph_x = 42.1875f32;
    let glyph_y = 104.25f32;

    for (ch, font_size) in test_cases {
        let gid = face
            .glyph_index(ch)
            .unwrap_or_else(|| panic!("Glyph index for character '{ch}' should exist"));

        // Step 1: extract_raster_glyph on real font
        let glyph = extract_raster_glyph(&font_bytes, 0, gid.0, font_size.round() as u16)
            .unwrap_or_else(|| panic!("Raster glyph for '{ch}' at size {font_size} should be extracted"));

        assert_eq!(&glyph.data[..8], &PNG_MAGIC_HEADER);
        assert!(glyph.width > 0.0);
        assert!(glyph.height > 0.0);

        // Step 2: Pass PNG bytes to BitmapGlyphCache::get_or_decode
        let key = BitmapCacheKey {
            font_key: 100,
            face_index: 0,
            glyph_id: gid.0,
            strike_ppem: glyph.strike_ppem,
        };

        let image = cache
            .get_or_decode(key, &glyph.data)
            .unwrap_or_else(|| panic!("Failed to decode PNG bytes for '{ch}'"));

        // Assert resulting peniko::ImageData
        assert_eq!(image.format, ImageFormat::Rgba8);
        assert_eq!(image.alpha_type, ImageAlphaType::Alpha);
        assert_eq!(image.width, glyph.width as u32);
        assert_eq!(image.height, glyph.height as u32);
        assert!(!image.data.is_empty(), "ImageData data buffer must be non-empty");
        assert_eq!(
            image.data.len(),
            (image.width * image.height * 4) as usize,
            "RGBA buffer length must equal width * height * 4"
        );
        let blob_id = image.data.id();

        // Cache hit test: verify subsequent retrieval preserves exact Blob ID
        let cached_image = cache
            .get_or_decode(key, &glyph.data)
            .expect("Cached image should be retrieved");
        assert_eq!(
            cached_image.data.id(),
            blob_id,
            "Cache hit must preserve Blob ID for GPU texture atlas reuse"
        );

        // Step 3: Calculate placement via compute_bitmap_placement_transform
        let (transform, bounds) =
            compute_bitmap_placement_transform(glyph_x, glyph_y, font_size, &glyph);

        // Step 4: Assert placement bounds match expected geometry
        let strike_scale = font_size / glyph.strike_ppem as f32;
        let expected_dest_x = glyph_x + glyph.origin_x * strike_scale;
        let expected_dest_y = glyph_y - (glyph.origin_y + glyph.height) * strike_scale;
        let expected_dest_w = glyph.width * strike_scale;
        let expected_dest_h = glyph.height * strike_scale;

        let tol = 1e-4;
        assert!(
            (bounds.x0 - expected_dest_x as f64).abs() < tol,
            "x0 mismatch for '{ch}': {} vs {}",
            bounds.x0,
            expected_dest_x
        );
        assert!(
            (bounds.y0 - expected_dest_y as f64).abs() < tol,
            "y0 mismatch for '{ch}': {} vs {}",
            bounds.y0,
            expected_dest_y
        );
        assert!(
            (bounds.width() - expected_dest_w as f64).abs() < tol,
            "width mismatch for '{ch}': {} vs {}",
            bounds.width(),
            expected_dest_w
        );
        assert!(
            (bounds.height() - expected_dest_h as f64).abs() < tol,
            "height mismatch for '{ch}': {} vs {}",
            bounds.height(),
            expected_dest_h
        );

        // Assert affine transform maps local bitmap corners exactly to bounds
        let top_left = transform * Point::new(0.0, 0.0);
        let bottom_right = transform * Point::new(glyph.width as f64, glyph.height as f64);
        assert!((top_left.x - bounds.x0).abs() < tol);
        assert!((top_left.y - bounds.y0).abs() < tol);
        assert!((bottom_right.x - bounds.x1).abs() < tol);
        assert!((bottom_right.y - bounds.y1).abs() < tol);
    }
}

#[test]
fn test_apple_color_emoji_coordinate_transform_invariance_across_scale_factors() {
    let Some(font_bytes) = load_apple_color_emoji() else {
        eprintln!("Skipping Apple Color Emoji contract test: file not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Face 0 should parse");
    let smile_gid = face.glyph_index('😀').expect("Smile glyph index exists");
    let glyph = extract_raster_glyph(&font_bytes, 0, smile_gid.0, 32).expect("Extracted glyph");

    let base_x = 50.0f32;
    let base_y = 100.0f32;
    let base_font_size = 32.0f32;

    let (base_transform, base_bounds) =
        compute_bitmap_placement_transform(base_x, base_y, base_font_size, &glyph);
    let base_coeffs = base_transform.as_coeffs();

    // Validate coordinate transform invariance across scale factors (1x, 2x Retina, 3x)
    for &scale in &[1.0f64, 2.0, 3.0] {
        let scaled_x = base_x * scale as f32;
        let scaled_y = base_y * scale as f32;
        let scaled_font_size = base_font_size * scale as f32;

        let (scaled_transform, scaled_bounds) =
            compute_bitmap_placement_transform(scaled_x, scaled_y, scaled_font_size, &glyph);
        let scaled_coeffs = scaled_transform.as_coeffs();

        let tol = 1e-4;
        assert!((scaled_bounds.x0 - base_bounds.x0 * scale).abs() < tol);
        assert!((scaled_bounds.y0 - base_bounds.y0 * scale).abs() < tol);
        assert!((scaled_bounds.x1 - base_bounds.x1 * scale).abs() < tol);
        assert!((scaled_bounds.y1 - base_bounds.y1 * scale).abs() < tol);
        assert!((scaled_bounds.width() - base_bounds.width() * scale).abs() < tol);
        assert!((scaled_bounds.height() - base_bounds.height() * scale).abs() < tol);

        // Affine matrix coefficients: [scale_x, 0, 0, scale_y, translate_x, translate_y]
        assert!((scaled_coeffs[0] - base_coeffs[0] * scale).abs() < tol);
        assert!((scaled_coeffs[1]).abs() < tol);
        assert!((scaled_coeffs[2]).abs() < tol);
        assert!((scaled_coeffs[3] - base_coeffs[3] * scale).abs() < tol);
        assert!((scaled_coeffs[4] - base_coeffs[4] * scale).abs() < tol);
        assert!((scaled_coeffs[5] - base_coeffs[5] * scale).abs() < tol);

        // Point transformations must scale proportionally without precision loss
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(glyph.width as f64, 0.0),
            Point::new(0.0, glyph.height as f64),
            Point::new(glyph.width as f64, glyph.height as f64),
            Point::new(glyph.width as f64 * 0.5, glyph.height as f64 * 0.5),
        ];

        for pt in corners {
            let base_pt = base_transform * pt;
            let scaled_pt = scaled_transform * pt;
            assert!((scaled_pt.x - base_pt.x * scale).abs() < tol);
            assert!((scaled_pt.y - base_pt.y * scale).abs() < tol);
        }
    }
}

#[test]
fn test_apple_color_emoji_subpixel_stability_and_monotonicity() {
    let Some(font_bytes) = load_apple_color_emoji() else {
        eprintln!("Skipping Apple Color Emoji contract test: file not found at {APPLE_COLOR_EMOJI_PATH}");
        return;
    };

    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Face 0 should parse");
    let smile_gid = face.glyph_index('😀').expect("Smile glyph index exists");
    let glyph = extract_raster_glyph(&font_bytes, 0, smile_gid.0, 32).expect("Extracted glyph");

    let glyph_x = 42.1875f32;
    let glyph_y = 104.25f32;

    // Arbitrary floating-point font sizes
    let font_sizes = [13.333f32, 16.5, 27.8125, 32.0, 48.75, 64.0, 104.25];

    for &fs in &font_sizes {
        let (transform, bounds) =
            compute_bitmap_placement_transform(glyph_x, glyph_y, fs, &glyph);

        // Validate finite and non-NaN
        assert!(bounds.x0.is_finite() && !bounds.x0.is_nan());
        assert!(bounds.y0.is_finite() && !bounds.y0.is_nan());
        assert!(bounds.x1.is_finite() && !bounds.x1.is_nan());
        assert!(bounds.y1.is_finite() && !bounds.y1.is_nan());
        assert!(bounds.width().is_finite() && !bounds.width().is_nan());
        assert!(bounds.height().is_finite() && !bounds.height().is_nan());
        assert!(bounds.width() > 0.0);
        assert!(bounds.height() > 0.0);

        for coeff in transform.as_coeffs() {
            assert!(coeff.is_finite() && !coeff.is_nan());
        }
    }

    // Monotonicity verification
    for window in font_sizes.windows(2) {
        let f_a = window[0];
        let f_b = window[1];

        let (t_a, b_a) = compute_bitmap_placement_transform(glyph_x, glyph_y, f_a, &glyph);
        let (t_b, b_b) = compute_bitmap_placement_transform(glyph_x, glyph_y, f_b, &glyph);

        assert!(b_a.width() < b_b.width());
        assert!(b_a.height() < b_b.height());
        assert!(t_a.as_coeffs()[0] < t_b.as_coeffs()[0]);
        assert!(t_a.as_coeffs()[3] < t_b.as_coeffs()[3]);
        assert!(b_a.y0 > b_b.y0);
        assert!(b_a.y1 < b_b.y1);
    }
}
