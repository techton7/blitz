use super::NormalizedRasterGlyph;

/// Extracts a raster bitmap glyph (specifically PNG formats such as Apple Color Emoji `sbix`)
/// from raw font bytes and normalizes its metrics and baseline.
pub fn extract_raster_glyph(
    font_bytes: &[u8],
    face_index: u32,
    glyph_id: u16,
    target_ppem: u16,
) -> Option<NormalizedRasterGlyph> {
    let face = ttf_parser::Face::parse(font_bytes, face_index).ok()?;
    let image = face.glyph_raster_image(ttf_parser::GlyphId(glyph_id), target_ppem)?;

    if image.width == 0 || image.height == 0 {
        return None;
    }

    if image.format != ttf_parser::RasterImageFormat::PNG {
        return None;
    }

    let origin_x = image.x as f32;
    let mut origin_y = image.y as f32;

    if image.y == 0 {
        let is_apple_color_emoji = face.names().into_iter().any(|name| {
            name.name_id == ttf_parser::name_id::FAMILY
                && (name
                    .to_string()
                    .map(|s| s.contains("Apple Color Emoji"))
                    .unwrap_or_else(|| {
                        String::from_utf8_lossy(name.name).contains("Apple Color Emoji")
                    }))
        });

        if is_apple_color_emoji {
            let descender = if let Some(b) = face.glyph_bounding_box(ttf_parser::GlyphId(glyph_id)) {
                if b.y_min < 0 {
                    (b.y_min as f32 * image.pixels_per_em as f32) / (face.units_per_em() as f32)
                } else {
                    (-100.0 * image.pixels_per_em as f32) / (face.units_per_em() as f32)
                }
            } else {
                (-100.0 * image.pixels_per_em as f32) / (face.units_per_em() as f32)
            };
            origin_y = descender;
        }
    }

    Some(NormalizedRasterGlyph {
        data: image.data.to_vec(),
        origin_x,
        origin_y,
        width: image.width as f32,
        height: image.height as f32,
        strike_ppem: image.pixels_per_em,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_invalid_font_returns_none() {
        let invalid_bytes = b"not a font";
        assert!(extract_raster_glyph(invalid_bytes, 0, 1, 32).is_none());
        assert!(extract_raster_glyph(&[], 0, 0, 16).is_none());
    }

    #[test]
    fn test_extract_non_emoji_font_returns_none() {
        // Test font without sbix table
        let no_sbix_font = create_test_font(&[], None);
        assert!(extract_raster_glyph(&no_sbix_font, 0, 0, 32).is_none());
        assert!(extract_raster_glyph(&no_sbix_font, 0, 1, 32).is_none());

        // Test standard outline font from repository assets
        let otf_bytes = include_bytes!("../../../blitz-dom/assets/moz-bullet-font.otf");
        assert!(extract_raster_glyph(otf_bytes, 0, 0, 16).is_none());
        assert!(extract_raster_glyph(otf_bytes, 0, 1, 32).is_none());
    }

    #[test]
    fn test_strike_selection_and_png_magic_header() {
        let font_bytes = create_test_font(
            &[
                (16, 16, 16),
                (32, 32, 32),
                (64, 64, 64),
            ],
            None,
        );

        // Exact strike selection: 16
        let g16 = extract_raster_glyph(&font_bytes, 0, 1, 16).expect("should extract strike 16");
        assert_eq!(g16.strike_ppem, 16);
        assert_eq!(g16.width, 16.0);
        assert_eq!(g16.height, 16.0);
        assert_eq!(&g16.data[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        // Exact strike selection: 32
        let g32 = extract_raster_glyph(&font_bytes, 0, 1, 32).expect("should extract strike 32");
        assert_eq!(g32.strike_ppem, 32);
        assert_eq!(g32.width, 32.0);
        assert_eq!(g32.height, 32.0);
        assert_eq!(&g32.data[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        // Exact strike selection: 64
        let g64 = extract_raster_glyph(&font_bytes, 0, 1, 64).expect("should extract strike 64");
        assert_eq!(g64.strike_ppem, 64);
        assert_eq!(g64.width, 64.0);
        assert_eq!(g64.height, 64.0);
        assert_eq!(&g64.data[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        // Best strike selection for intermediate requests:
        // Request 10 -> chooses strike 16 (first strike >= 10)
        let g10 = extract_raster_glyph(&font_bytes, 0, 1, 10).expect("should match strike 16");
        assert_eq!(g10.strike_ppem, 16);

        // Request 20 -> chooses strike 32 (first strike >= 20)
        let g20 = extract_raster_glyph(&font_bytes, 0, 1, 20).expect("should match strike 32");
        assert_eq!(g20.strike_ppem, 32);

        // Request 50 -> chooses strike 64 (first strike >= 50)
        let g50 = extract_raster_glyph(&font_bytes, 0, 1, 50).expect("should match strike 64");
        assert_eq!(g50.strike_ppem, 64);

        // Request 128 -> chooses strike 64 (largest strike when request exceeds all strikes)
        let g128 = extract_raster_glyph(&font_bytes, 0, 1, 128).expect("should match max strike 64");
        assert_eq!(g128.strike_ppem, 64);
    }

    #[test]
    fn test_zero_width_and_empty_glyph_handling() {
        // Glyph 0 is empty (start == end offset)
        let font_bytes = create_test_font(&[(32, 32, 32)], None);
        assert!(extract_raster_glyph(&font_bytes, 0, 0, 32).is_none());

        // Out of bounds glyph id
        assert!(extract_raster_glyph(&font_bytes, 0, 999, 32).is_none());

        // Glyph with zero-width PNG image:
        let zero_w_font = create_test_font(&[(32, 0, 32)], None);
        assert!(extract_raster_glyph(&zero_w_font, 0, 1, 32).is_none());

        // Glyph with zero-height PNG image:
        let zero_h_font = create_test_font(&[(32, 32, 0)], None);
        assert!(extract_raster_glyph(&zero_h_font, 0, 1, 32).is_none());
    }

    #[test]
    fn test_apple_color_emoji_descender_normalization() {
        // Synthetic font named "Apple Color Emoji" (falls back to -100 units descender)
        let font_apple = create_test_font(
            &[(32, 32, 32)],
            Some("Apple Color Emoji"),
        );
        let g_apple = extract_raster_glyph(&font_apple, 0, 1, 32)
            .expect("should extract raster glyph");
        // Expected origin_y = (-100.0 * 32.0) / 1000.0 = -3.2
        assert_eq!(g_apple.origin_y, -3.2);
        assert_eq!(g_apple.origin_x, 0.0);

        // Non-Apple font with image.y == 0 should NOT modify origin_y (remains 0.0)
        let font_other = create_test_font(
            &[(32, 32, 32)],
            Some("Custom Emoji Font"),
        );
        let g_other = extract_raster_glyph(&font_other, 0, 1, 32)
            .expect("should extract raster glyph");
        assert_eq!(g_other.origin_y, 0.0);
    }

    fn create_test_font(
        strikes: &[(u16, u16, u16)],
        family_name: Option<&str>,
    ) -> Vec<u8> {
        let mut head = Vec::new();
        head.extend_from_slice(&0x00010000u32.to_be_bytes()); // version
        head.extend_from_slice(&0x00010000u32.to_be_bytes()); // fontRevision
        head.extend_from_slice(&0u32.to_be_bytes()); // checkSumAdjustment
        head.extend_from_slice(&0x5F0F3CF5u32.to_be_bytes()); // magicNumber
        head.extend_from_slice(&0u16.to_be_bytes()); // flags
        head.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
        head.extend_from_slice(&0i64.to_be_bytes()); // created
        head.extend_from_slice(&0i64.to_be_bytes()); // modified
        head.extend_from_slice(&0i16.to_be_bytes()); // xMin
        head.extend_from_slice(&(-200i16).to_be_bytes()); // yMin
        head.extend_from_slice(&1000i16.to_be_bytes()); // xMax
        head.extend_from_slice(&800i16.to_be_bytes()); // yMax
        head.extend_from_slice(&0u16.to_be_bytes()); // macStyle
        head.extend_from_slice(&6u16.to_be_bytes()); // lowestRecPPEM
        head.extend_from_slice(&2i16.to_be_bytes()); // fontDirectionHint
        head.extend_from_slice(&0i16.to_be_bytes()); // indexToLocFormat (0 for short)
        head.extend_from_slice(&0i16.to_be_bytes()); // glyphDataFormat

        let mut hhea = Vec::new();
        hhea.extend_from_slice(&0x00010000u32.to_be_bytes()); // version
        hhea.extend_from_slice(&800i16.to_be_bytes()); // ascender
        hhea.extend_from_slice(&(-200i16).to_be_bytes()); // descender
        hhea.extend_from_slice(&0i16.to_be_bytes()); // lineGap
        hhea.extend_from_slice(&1000u16.to_be_bytes()); // advanceWidthMax
        hhea.extend_from_slice(&0i16.to_be_bytes()); // minLeftSideBearing
        hhea.extend_from_slice(&0i16.to_be_bytes()); // minRightSideBearing
        hhea.extend_from_slice(&1000i16.to_be_bytes()); // xMaxExtent
        hhea.extend_from_slice(&1i16.to_be_bytes()); // caretSlopeRise
        hhea.extend_from_slice(&0i16.to_be_bytes()); // caretSlopeRun
        hhea.extend_from_slice(&0i16.to_be_bytes()); // caretOffset
        hhea.extend_from_slice(&[0u8; 8]); // 4 reserved i16
        hhea.extend_from_slice(&0i16.to_be_bytes()); // metricDataFormat
        hhea.extend_from_slice(&1u16.to_be_bytes()); // numberOfHMetrics

        let mut maxp = Vec::new();
        maxp.extend_from_slice(&0x00005000u32.to_be_bytes()); // version 0.5
        maxp.extend_from_slice(&2u16.to_be_bytes()); // numGlyphs = 2

        // Optional name table
        let mut name_table = Vec::new();
        if let Some(family) = family_name {
            name_table.extend_from_slice(&0u16.to_be_bytes()); // format 0
            name_table.extend_from_slice(&1u16.to_be_bytes()); // count 1
            name_table.extend_from_slice(&18u16.to_be_bytes()); // stringOffset = 6 + 12 = 18
            name_table.extend_from_slice(&1u16.to_be_bytes()); // platformId = 1 (Macintosh)
            name_table.extend_from_slice(&0u16.to_be_bytes()); // encodingId = 0 (Roman)
            name_table.extend_from_slice(&0u16.to_be_bytes()); // languageId = 0 (English)
            name_table.extend_from_slice(&1u16.to_be_bytes()); // nameId = 1 (Family name)
            name_table.extend_from_slice(&(family.len() as u16).to_be_bytes()); // length
            name_table.extend_from_slice(&0u16.to_be_bytes()); // offset = 0
            name_table.extend_from_slice(family.as_bytes());
        }

        // Optional sbix table
        let mut sbix = Vec::new();
        if !strikes.is_empty() {
            sbix.extend_from_slice(&1u16.to_be_bytes()); // version
            sbix.extend_from_slice(&0u16.to_be_bytes()); // flags
            sbix.extend_from_slice(&(strikes.len() as u32).to_be_bytes()); // number of strikes

            let mut strike_datas = Vec::new();
            for &(ppem, w, h) in strikes {
                let mut sdata = Vec::new();
                sdata.extend_from_slice(&ppem.to_be_bytes());
                sdata.extend_from_slice(&72u16.to_be_bytes()); // ppi
                let glyph_0_offset = 4 + 3 * 4; // 16
                let glyph_1_offset = glyph_0_offset; // glyph 0 is empty
                let mut glyph_1_data = Vec::new();
                glyph_1_data.extend_from_slice(&0i16.to_be_bytes()); // x
                glyph_1_data.extend_from_slice(&0i16.to_be_bytes()); // y
                glyph_1_data.extend_from_slice(b"png "); // type tag
                glyph_1_data.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
                glyph_1_data.extend_from_slice(&13u32.to_be_bytes());
                glyph_1_data.extend_from_slice(b"IHDR");
                glyph_1_data.extend_from_slice(&(w as u32).to_be_bytes());
                glyph_1_data.extend_from_slice(&(h as u32).to_be_bytes());

                let glyph_end_offset = glyph_1_offset + glyph_1_data.len();

                sdata.extend_from_slice(&(glyph_0_offset as u32).to_be_bytes());
                sdata.extend_from_slice(&(glyph_1_offset as u32).to_be_bytes());
                sdata.extend_from_slice(&(glyph_end_offset as u32).to_be_bytes());
                sdata.extend_from_slice(&glyph_1_data);

                strike_datas.push(sdata);
            }

            let mut current_strike_offset = 8 + 4 * strikes.len();
            for sdata in &strike_datas {
                sbix.extend_from_slice(&(current_strike_offset as u32).to_be_bytes());
                current_strike_offset += sdata.len();
            }
            for sdata in &strike_datas {
                sbix.extend_from_slice(sdata);
            }
        }

        fn pad4(buf: &mut Vec<u8>) {
            let rem = buf.len() % 4;
            if rem != 0 {
                buf.resize(buf.len() + (4 - rem), 0);
            }
        }
        pad4(&mut head);
        pad4(&mut hhea);
        pad4(&mut maxp);
        if !name_table.is_empty() {
            pad4(&mut name_table);
        }
        if !sbix.is_empty() {
            pad4(&mut sbix);
        }

        let mut tables = vec![
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"maxp", maxp),
        ];
        if !name_table.is_empty() {
            tables.push((*b"name", name_table));
        }
        if !sbix.is_empty() {
            tables.push((*b"sbix", sbix));
        }

        tables.sort_by_key(|(tag, _)| *tag);

        let num_tables = tables.len() as u16;
        let mut font = Vec::new();
        font.extend_from_slice(&0x00010000u32.to_be_bytes()); // sfnt
        font.extend_from_slice(&num_tables.to_be_bytes());
        font.extend_from_slice(&0u16.to_be_bytes()); // searchRange
        font.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        font.extend_from_slice(&0u16.to_be_bytes()); // rangeShift

        let mut offset = 12 + 16 * tables.len();
        for (tag, data) in &tables {
            font.extend_from_slice(tag);
            font.extend_from_slice(&0u32.to_be_bytes()); // checksum
            font.extend_from_slice(&(offset as u32).to_be_bytes());
            font.extend_from_slice(&(data.len() as u32).to_be_bytes());
            offset += data.len();
        }

        for (_, data) in &tables {
            font.extend_from_slice(data);
        }

        font
    }
}
