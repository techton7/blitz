use super::NormalizedRasterGlyph;

/// Computes the affine placement transform and world bounding rectangle for a raster bitmap glyph.
///
/// In typographic coordinate systems, `glyph_x` and `glyph_y` designate the baseline origin point.
/// Because raster glyphs (e.g. from Apple Color Emoji `sbix`) are provided at fixed strike sizes,
/// they must be scaled by `strike_scale = font_size / glyph.strike_ppem`.
///
/// The bitmap origin is offset from the baseline:
/// - Horizontal: `dest_x = glyph_x + glyph.origin_x * strike_scale`
/// - Vertical: in downward-increasing screen space, `dest_y = glyph_y - (glyph.origin_y + glyph.height) * strike_scale`
///
/// Returns:
/// - `kurbo::Affine`: The transform to map the local bitmap `[0, width] x [0, height]` into canvas space.
/// - `kurbo::Rect`: The bounding box in canvas space `[dest_x, dest_y, dest_x + dest_w, dest_y + dest_h]`.
pub fn compute_bitmap_placement_transform(
    glyph_x: f32,
    glyph_y: f32,
    font_size: f32,
    glyph: &NormalizedRasterGlyph,
) -> (kurbo::Affine, kurbo::Rect) {
    let strike_scale = font_size / glyph.strike_ppem as f32;
    let dest_x = glyph_x + glyph.origin_x * strike_scale;
    let dest_y = glyph_y - (glyph.origin_y + glyph.height) * strike_scale;
    let dest_w = glyph.width * strike_scale;
    let dest_h = glyph.height * strike_scale;
    let transform = kurbo::Affine::translate((dest_x as f64, dest_y as f64))
        * kurbo::Affine::scale(strike_scale as f64);
    let bounds = kurbo::Rect::new(
        dest_x as f64,
        dest_y as f64,
        (dest_x + dest_w) as f64,
        (dest_y + dest_h) as f64,
    );
    (transform, bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    fn sample_glyph(origin_x: f32, origin_y: f32, width: f32, height: f32, strike_ppem: u16) -> NormalizedRasterGlyph {
        NormalizedRasterGlyph {
            data: Vec::new(),
            origin_x,
            origin_y,
            width,
            height,
            strike_ppem,
        }
    }

    #[test]
    fn test_placement_at_1x_scale() {
        // 1x scale: font_size matches strike_ppem exactly.
        let glyph = sample_glyph(0.0, 0.0, 32.0, 32.0, 32);
        let glyph_x = 100.0;
        let glyph_y = 200.0;
        let font_size = 32.0;

        let (transform, bounds) = compute_bitmap_placement_transform(glyph_x, glyph_y, font_size, &glyph);

        // dest_x = 100 + 0 * 1 = 100
        // dest_y = 200 - (0 + 32) * 1 = 168
        assert_eq!(bounds.x0, 100.0);
        assert_eq!(bounds.y0, 168.0);
        assert_eq!(bounds.x1, 132.0);
        assert_eq!(bounds.y1, 200.0);
        assert_eq!(bounds.width(), 32.0);
        assert_eq!(bounds.height(), 32.0);

        // Verify transform maps local bitmap corners to bounds:
        let top_left = transform * Point::new(0.0, 0.0);
        let bottom_right = transform * Point::new(32.0, 32.0);
        assert_eq!(top_left, Point::new(100.0, 168.0));
        assert_eq!(bottom_right, Point::new(132.0, 200.0));
    }

    #[test]
    fn test_placement_at_2x_scale() {
        // 2x scale: font_size is double the strike_ppem.
        let glyph = sample_glyph(2.0, -4.0, 32.0, 32.0, 32);
        let glyph_x = 50.0;
        let glyph_y = 150.0;
        let font_size = 64.0; // strike_scale = 2.0

        let (transform, bounds) = compute_bitmap_placement_transform(glyph_x, glyph_y, font_size, &glyph);

        // strike_scale = 2.0
        // dest_x = 50 + 2 * 2 = 54.0
        // dest_y = 150 - (-4 + 32) * 2 = 150 - 56 = 94.0
        // dest_w = 32 * 2 = 64.0
        // dest_h = 32 * 2 = 64.0
        assert_eq!(bounds.x0, 54.0);
        assert_eq!(bounds.y0, 94.0);
        assert_eq!(bounds.x1, 118.0);
        assert_eq!(bounds.y1, 158.0);
        assert_eq!(bounds.width(), 64.0);
        assert_eq!(bounds.height(), 64.0);

        let top_left = transform * Point::new(0.0, 0.0);
        let bottom_right = transform * Point::new(32.0, 32.0);
        assert_eq!(top_left, Point::new(54.0, 94.0));
        assert_eq!(bottom_right, Point::new(118.0, 158.0));
    }

    #[test]
    fn test_placement_at_3x_scale() {
        // 3x scale: font_size is triple the strike_ppem.
        let glyph = sample_glyph(-1.0, 1.0, 20.0, 25.0, 20);
        let glyph_x = 10.0;
        let glyph_y = 20.0;
        let font_size = 60.0; // strike_scale = 3.0

        let (transform, bounds) = compute_bitmap_placement_transform(glyph_x, glyph_y, font_size, &glyph);

        // strike_scale = 3.0
        // dest_x = 10 + (-1) * 3 = 7.0
        // dest_y = 20 - (1 + 25) * 3 = 20 - 78 = -58.0
        // dest_w = 20 * 3 = 60.0
        // dest_h = 25 * 3 = 75.0
        assert_eq!(bounds.x0, 7.0);
        assert_eq!(bounds.y0, -58.0);
        assert_eq!(bounds.x1, 67.0);
        assert_eq!(bounds.y1, 17.0);
        assert_eq!(bounds.width(), 60.0);
        assert_eq!(bounds.height(), 75.0);

        let top_left = transform * Point::new(0.0, 0.0);
        let bottom_right = transform * Point::new(20.0, 25.0);
        assert_eq!(top_left, Point::new(7.0, -58.0));
        assert_eq!(bottom_right, Point::new(67.0, 17.0));
    }

    #[test]
    fn test_placement_fractional_scale() {
        // Fractional scale: strike_ppem = 32, font_size = 24 -> scale = 0.75
        let glyph = sample_glyph(0.0, 0.0, 32.0, 32.0, 32);
        let glyph_x = 0.0;
        let glyph_y = 100.0;
        let font_size = 24.0; // strike_scale = 0.75

        let (transform, bounds) = compute_bitmap_placement_transform(glyph_x, glyph_y, font_size, &glyph);

        // dest_x = 0
        // dest_y = 100 - 32 * 0.75 = 76.0
        // dest_w = 32 * 0.75 = 24.0
        // dest_h = 32 * 0.75 = 24.0
        assert_eq!(bounds.x0, 0.0);
        assert_eq!(bounds.y0, 76.0);
        assert_eq!(bounds.x1, 24.0);
        assert_eq!(bounds.y1, 100.0);

        let top_left = transform * Point::new(0.0, 0.0);
        let bottom_right = transform * Point::new(32.0, 32.0);
        assert_eq!(top_left, Point::new(0.0, 76.0));
        assert_eq!(bottom_right, Point::new(24.0, 100.0));
    }

    #[test]
    fn test_placement_descender_below_baseline() {
        // When origin_y is negative, the glyph extends below the baseline.
        // glyph_y is baseline (e.g. 100).
        // origin_y = -5.0, height = 40.0.
        // dest_y = 100 - (-5 + 40) * 1 = 100 - 35 = 65.0.
        // bottom = dest_y + height = 65 + 40 = 105.0 (5px below baseline).
        let glyph = sample_glyph(0.0, -5.0, 40.0, 40.0, 40);
        let (_, bounds) = compute_bitmap_placement_transform(0.0, 100.0, 40.0, &glyph);

        assert_eq!(bounds.y0, 65.0);
        assert_eq!(bounds.y1, 105.0);
    }

    #[test]
    fn test_scale_factor_invariance_mathematical() {
        // Mathematical invariance test verifying that the placement transform scales
        // dimensions and positions proportionally without precision loss across scale factors:
        // 1x, 2x Retina, 3x, and fractional scales.
        let glyphs = [
            // Standard emoji glyph with descender:
            sample_glyph(0.0, -4.0, 32.0, 32.0, 32),
            // Offset non-square glyph:
            sample_glyph(2.5, -3.5, 28.0, 36.0, 32),
            // Asymmetric glyph:
            sample_glyph(-1.5, 2.0, 40.0, 24.0, 20),
        ];

        let base_x = 42.1875f32;
        let base_y = 104.25f32;
        let base_font_size = 16.0f32;

        let scale_factors: &[f64] = &[1.0, 1.5, 2.0, 2.5, 3.0, 4.0];

        for glyph in &glyphs {
            let (base_transform, base_bounds) =
                compute_bitmap_placement_transform(base_x, base_y, base_font_size, glyph);
            let base_coeffs = base_transform.as_coeffs();

            for &scale in scale_factors {
                let scaled_x = base_x * scale as f32;
                let scaled_y = base_y * scale as f32;
                let scaled_font_size = base_font_size * scale as f32;

                let (scaled_transform, scaled_bounds) =
                    compute_bitmap_placement_transform(scaled_x, scaled_y, scaled_font_size, glyph);
                let scaled_coeffs = scaled_transform.as_coeffs();

                // 1. Validate bounding box coordinates scale proportionally:
                let tol = 1e-4;
                assert!(
                    (scaled_bounds.x0 - base_bounds.x0 * scale).abs() < tol,
                    "x0 invariance failed for scale {scale}: {} vs {}",
                    scaled_bounds.x0,
                    base_bounds.x0 * scale
                );
                assert!(
                    (scaled_bounds.y0 - base_bounds.y0 * scale).abs() < tol,
                    "y0 invariance failed for scale {scale}: {} vs {}",
                    scaled_bounds.y0,
                    base_bounds.y0 * scale
                );
                assert!(
                    (scaled_bounds.x1 - base_bounds.x1 * scale).abs() < tol,
                    "x1 invariance failed for scale {scale}: {} vs {}",
                    scaled_bounds.x1,
                    base_bounds.x1 * scale
                );
                assert!(
                    (scaled_bounds.y1 - base_bounds.y1 * scale).abs() < tol,
                    "y1 invariance failed for scale {scale}: {} vs {}",
                    scaled_bounds.y1,
                    base_bounds.y1 * scale
                );
                assert!(
                    (scaled_bounds.width() - base_bounds.width() * scale).abs() < tol,
                    "width invariance failed for scale {scale}"
                );
                assert!(
                    (scaled_bounds.height() - base_bounds.height() * scale).abs() < tol,
                    "height invariance failed for scale {scale}"
                );

                // 2. Validate affine transform coefficients scale proportionally:
                // Affine matrix: [scale_x, 0, 0, scale_y, translate_x, translate_y]
                assert!((scaled_coeffs[0] - base_coeffs[0] * scale).abs() < tol);
                assert!((scaled_coeffs[1]).abs() < tol);
                assert!((scaled_coeffs[2]).abs() < tol);
                assert!((scaled_coeffs[3] - base_coeffs[3] * scale).abs() < tol);
                assert!((scaled_coeffs[4] - base_coeffs[4] * scale).abs() < tol);
                assert!((scaled_coeffs[5] - base_coeffs[5] * scale).abs() < tol);

                // 3. Validate transformed points scale proportionally:
                let test_points = [
                    Point::new(0.0, 0.0),
                    Point::new(glyph.width as f64, 0.0),
                    Point::new(0.0, glyph.height as f64),
                    Point::new(glyph.width as f64, glyph.height as f64),
                    Point::new(glyph.width as f64 * 0.5, glyph.height as f64 * 0.5),
                    Point::new(glyph.width as f64 * 0.333, glyph.height as f64 * 0.777),
                ];

                for pt in test_points {
                    let base_pt = base_transform * pt;
                    let scaled_pt = scaled_transform * pt;

                    assert!(
                        (scaled_pt.x - base_pt.x * scale).abs() < tol,
                        "point x invariance failed for pt {pt:?} at scale {scale}"
                    );
                    assert!(
                        (scaled_pt.y - base_pt.y * scale).abs() < tol,
                        "point y invariance failed for pt {pt:?} at scale {scale}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_subpixel_position_stability_arbitrary_coordinates() {
        // Arbitrary floating-point font sizes and baseline coordinates
        let font_sizes = [
            0.01f32, 0.5, 7.25, 12.0, 13.333, 16.0, 16.5, 21.875, 27.8125, 32.0,
            48.125, 64.0, 72.3333, 100.0, 104.25, 128.5, 256.0, 1024.0,
        ];
        let baseline_xs = [-150.333f32, -0.75, 0.0, 0.125, 10.333, 42.1875, 104.25, 512.667];
        let baseline_ys = [-50.125f32, 0.0, 0.333, 15.5, 104.25, 200.75, 999.999];

        let glyph = sample_glyph(2.5, -4.0, 32.0, 32.0, 32);

        for &fs in &font_sizes {
            for &bx in &baseline_xs {
                for &by in &baseline_ys {
                    let (transform, bounds) = compute_bitmap_placement_transform(bx, by, fs, &glyph);

                    // 1. Bounds coordinates and dimensions must be finite and non-NaN
                    assert!(bounds.x0.is_finite() && !bounds.x0.is_nan());
                    assert!(bounds.y0.is_finite() && !bounds.y0.is_nan());
                    assert!(bounds.x1.is_finite() && !bounds.x1.is_nan());
                    assert!(bounds.y1.is_finite() && !bounds.y1.is_nan());
                    assert!(bounds.width().is_finite() && !bounds.width().is_nan());
                    assert!(bounds.height().is_finite() && !bounds.height().is_nan());
                    assert!(bounds.width() > 0.0);
                    assert!(bounds.height() > 0.0);

                    // 2. Matrix coefficients must be finite and non-NaN
                    for coeff in transform.as_coeffs() {
                        assert!(coeff.is_finite() && !coeff.is_nan());
                    }

                    // 3. Geometric consistency: width == x1 - x0, height == y1 - y0
                    assert!((bounds.x1 - bounds.x0 - bounds.width()).abs() < 1e-6);
                    assert!((bounds.y1 - bounds.y0 - bounds.height()).abs() < 1e-6);

                    // 4. Subpixel shift linearity:
                    let delta = 0.375f32;
                    let (_, shifted_bounds) =
                        compute_bitmap_placement_transform(bx + delta, by + delta, fs, &glyph);
                    assert!((shifted_bounds.x0 - (bounds.x0 + delta as f64)).abs() < 1e-6);
                    assert!((shifted_bounds.y0 - (bounds.y0 + delta as f64)).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn test_font_size_strict_monotonicity() {
        let glyph = sample_glyph(0.0, -4.0, 32.0, 32.0, 32);
        let glyph_x = 104.25f32;
        let glyph_y = 200.75f32;

        let font_sizes = [
            0.1f32, 0.5, 1.0, 5.5, 10.0, 13.333, 16.0, 16.5, 20.0, 24.0,
            27.8125, 32.0, 48.0, 64.0, 96.0, 104.25, 128.0, 256.0,
        ];

        for window in font_sizes.windows(2) {
            let f_a = window[0];
            let f_b = window[1];
            assert!(f_a < f_b);

            let (transform_a, bounds_a) =
                compute_bitmap_placement_transform(glyph_x, glyph_y, f_a, &glyph);
            let (transform_b, bounds_b) =
                compute_bitmap_placement_transform(glyph_x, glyph_y, f_b, &glyph);

            // Bounds width and height must be strictly monotonic
            assert!(
                bounds_a.width() < bounds_b.width(),
                "width should strictly increase: {} vs {}",
                bounds_a.width(),
                bounds_b.width()
            );
            assert!(
                bounds_a.height() < bounds_b.height(),
                "height should strictly increase: {} vs {}",
                bounds_a.height(),
                bounds_b.height()
            );

            // Matrix scale coefficients must be strictly monotonic
            let coeffs_a = transform_a.as_coeffs();
            let coeffs_b = transform_b.as_coeffs();
            assert!(
                coeffs_a[0] < coeffs_b[0],
                "scale_x should strictly increase: {} vs {}",
                coeffs_a[0],
                coeffs_b[0]
            );
            assert!(
                coeffs_a[3] < coeffs_b[3],
                "scale_y should strictly increase: {} vs {}",
                coeffs_a[3],
                coeffs_b[3]
            );

            // Top bounds y0 moves strictly higher in canvas space (smaller Y)
            assert!(
                bounds_a.y0 > bounds_b.y0,
                "top edge y0 should strictly decrease: {} vs {}",
                bounds_a.y0,
                bounds_b.y0
            );

            // Descender bottom bounds y1 extends strictly lower below baseline (larger Y)
            assert!(
                bounds_a.y1 < bounds_b.y1,
                "bottom edge y1 should strictly increase: {} vs {}",
                bounds_a.y1,
                bounds_b.y1
            );
        }
    }
}
