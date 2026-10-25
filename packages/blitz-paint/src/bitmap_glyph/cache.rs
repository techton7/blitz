use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

pub const DEFAULT_MAX_ENTRIES: usize = 512;
pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024; // 16 MB

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitmapCacheKey {
    pub font_key: u64,
    pub face_index: u32,
    pub glyph_id: u16,
    pub strike_ppem: u16,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    image: ImageData,
    byte_size: usize,
    last_access: u64,
}

/// An LRU cache for decoded bitmap glyph images.
///
/// Bounded by both entry count (default 512) and memory footprint (default 16MB).
/// Ensures that cached images preserve their underlying [`Blob`] ID across accesses,
/// enabling Vello GPU texture atlas reuse.
#[derive(Debug)]
pub struct BitmapGlyphCache {
    entries: HashMap<BitmapCacheKey, CacheEntry>,
    max_entries: usize,
    max_bytes: usize,
    current_bytes: usize,
    access_counter: u64,
}

impl Default for BitmapGlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_CACHE: std::sync::OnceLock<std::sync::Mutex<BitmapGlyphCache>> =
    std::sync::OnceLock::new();

impl BitmapGlyphCache {
    /// Returns a reference to the process-global bitmap glyph cache.
    pub fn global() -> &'static std::sync::Mutex<BitmapGlyphCache> {
        GLOBAL_CACHE.get_or_init(|| std::sync::Mutex::new(BitmapGlyphCache::new()))
    }

    /// Creates a new cache with default bounds (512 entries, 16 MB).
    pub fn new() -> Self {
        Self::with_bounds(DEFAULT_MAX_ENTRIES, DEFAULT_MAX_BYTES)
    }

    /// Creates a new cache with custom entry and byte capacity bounds.
    pub fn with_bounds(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            max_bytes,
            current_bytes: 0,
            access_counter: 0,
        }
    }

    /// Returns the number of cached bitmap glyphs.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the total bytes consumed by cached RGBA image buffers.
    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    /// Retrieves an image from the cache or decodes the provided PNG bytes, inserting it into cache.
    ///
    /// Preserves the [`Blob`] identity for subsequent accesses so downstream renderers
    /// (e.g. Vello GPU texture atlases) can avoid texture re-uploads.
    pub fn get_or_decode(&mut self, key: BitmapCacheKey, png_bytes: &[u8]) -> Option<ImageData> {
        self.access_counter = self.access_counter.wrapping_add(1);

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_access = self.access_counter;
            return Some(entry.image.clone());
        }

        let image = decode_png_to_image_data(png_bytes)?;
        let byte_size = image.data.len();

        // Evict entries while capacity limits are exceeded
        while (self.entries.len() >= self.max_entries
            || self.current_bytes + byte_size > self.max_bytes)
            && !self.entries.is_empty()
        {
            let oldest_key = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(k, _)| *k);

            if let Some(oldest_key) = oldest_key {
                if let Some(removed) = self.entries.remove(&oldest_key) {
                    self.current_bytes = self.current_bytes.saturating_sub(removed.byte_size);
                }
            } else {
                break;
            }
        }

        // Only store if the single entry fits within max_bytes and max_entries > 0
        if byte_size <= self.max_bytes && self.entries.len() < self.max_entries {
            self.current_bytes += byte_size;
            self.entries.insert(
                key,
                CacheEntry {
                    image: image.clone(),
                    byte_size,
                    last_access: self.access_counter,
                },
            );
        }

        Some(image)
    }
}

/// Decodes raw PNG bytes into a standard 8-bit RGBA [`ImageData`].
///
/// Handles RGBA, RGB (converted to RGBA with alpha = 255), Grayscale,
/// and Grayscale with Alpha. Strips 16-bit channels to 8-bit.
pub fn decode_png_to_image_data(png_bytes: &[u8]) -> Option<ImageData> {
    let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let output_info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..output_info.buffer_size()];

    let width = output_info.width;
    let height = output_info.height;

    let rgba = match output_info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            let (chunks, _) = data.as_chunks::<3>();
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for chunk in chunks {
                rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            rgba
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for &y in data {
                rgba.extend_from_slice(&[y, y, y, 255]);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            let (chunks, _) = data.as_chunks::<2>();
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for chunk in chunks {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            rgba
        }
        png::ColorType::Indexed => return None,
    };

    Some(ImageData {
        data: Blob::new(Arc::new(rgba)),
        format: ImageFormat::Rgba8,
        width,
        height,
        alpha_type: ImageAlphaType::Alpha,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_png(
        width: u32,
        height: u32,
        color_type: png::ColorType,
        raw_pixels: &[u8],
    ) -> Vec<u8> {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
            encoder.set_color(color_type);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("write header");
            writer
                .write_image_data(raw_pixels)
                .expect("write image data");
        }
        png_bytes
    }

    #[test]
    fn test_decode_rgba8_png() {
        let pixels = [
            255, 0, 0, 255, // Red
            0, 255, 0, 128, // Green (half alpha)
            0, 0, 255, 255, // Blue
            255, 255, 0, 200, // Yellow
        ];
        let png = create_test_png(2, 2, png::ColorType::Rgba, &pixels);

        let img = decode_png_to_image_data(&png).expect("valid png should decode");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.format, ImageFormat::Rgba8);
        assert_eq!(img.alpha_type, ImageAlphaType::Alpha);
        assert_eq!(img.data.as_ref(), &pixels);
    }

    #[test]
    fn test_decode_rgb8_png() {
        let pixels = [
            255, 0, 0, // Red
            0, 255, 0, // Green
        ];
        let png = create_test_png(2, 1, png::ColorType::Rgb, &pixels);

        let img = decode_png_to_image_data(&png).expect("valid rgb png should decode");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 1);
        let expected = [255, 0, 0, 255, 0, 255, 0, 255];
        assert_eq!(img.data.as_ref(), &expected);
    }

    #[test]
    fn test_decode_grayscale_png() {
        let pixels = [100, 200];
        let png = create_test_png(2, 1, png::ColorType::Grayscale, &pixels);

        let img = decode_png_to_image_data(&png).expect("valid grayscale png should decode");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 1);
        let expected = [100, 100, 100, 255, 200, 200, 200, 255];
        assert_eq!(img.data.as_ref(), &expected);
    }

    #[test]
    fn test_decode_grayscale_alpha_png() {
        let pixels = [120, 50]; // 1 pixel: gray=120, alpha=50
        let png = create_test_png(1, 1, png::ColorType::GrayscaleAlpha, &pixels);

        let img = decode_png_to_image_data(&png).expect("valid grayscale alpha png should decode");
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 1);
        let expected = [120, 120, 120, 50];
        assert_eq!(img.data.as_ref(), &expected);
    }

    #[test]
    fn test_decode_invalid_png_returns_none() {
        assert!(decode_png_to_image_data(b"not a png").is_none());
        assert!(decode_png_to_image_data(&[]).is_none());
        assert!(decode_png_to_image_data(&[0x89, 0x50, 0x4E, 0x47]).is_none());
    }

    #[test]
    fn test_cache_hit_preserves_blob_id() {
        let mut cache = BitmapGlyphCache::new();
        let key = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 42,
            strike_ppem: 32,
        };

        let pixels = [10, 20, 30, 40];
        let png = create_test_png(1, 1, png::ColorType::Rgba, &pixels);

        let img1 = cache
            .get_or_decode(key, &png)
            .expect("first decode should succeed");
        let img2 = cache
            .get_or_decode(key, &png)
            .expect("cache hit should succeed");

        // CRITICAL: blob ID must match for Vello GPU atlas reuse
        assert_eq!(img1.data.id(), img2.data.id());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_lru_entry_count_eviction() {
        // Max 3 entries
        let mut cache = BitmapGlyphCache::with_bounds(3, 1024 * 1024);

        let png = create_test_png(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4]);

        let key1 = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 1,
            strike_ppem: 16,
        };
        let key2 = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 2,
            strike_ppem: 16,
        };
        let key3 = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 3,
            strike_ppem: 16,
        };
        let key4 = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 4,
            strike_ppem: 16,
        };

        let img1 = cache.get_or_decode(key1, &png).unwrap();
        let _ = cache.get_or_decode(key2, &png).unwrap();
        let _ = cache.get_or_decode(key3, &png).unwrap();
        assert_eq!(cache.len(), 3);

        // Access key1 again to update its tick (making key2 the LRU entry)
        let img1_again = cache.get_or_decode(key1, &png).unwrap();
        assert_eq!(img1.data.id(), img1_again.data.id());

        // Insert key4 -> key2 should be evicted
        let _ = cache.get_or_decode(key4, &png).unwrap();
        assert_eq!(cache.len(), 3);

        // Verify key1 is still in cache (id matches without new allocation)
        let img1_check = cache.get_or_decode(key1, &png).unwrap();
        assert_eq!(img1.data.id(), img1_check.data.id());

        // Now if key2 is re-requested, it gets re-decoded and receives a NEW blob ID
        let img2_new = cache.get_or_decode(key2, &png).unwrap();
        // key3 was least recently accessed now (since key1 and key4 and key1 were accessed)
        // key3 got evicted when key2 was inserted!
        assert_eq!(cache.len(), 3);
        assert_eq!(img2_new.data.as_ref(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_cache_byte_size_eviction() {
        // 1x1 RGBA is 4 bytes.
        // Limit cache to 10 bytes: can hold at most 2 images (8 bytes). A 3rd image (4 bytes)
        // will exceed 10 bytes, causing eviction of the oldest image.
        let mut cache = BitmapGlyphCache::with_bounds(100, 10);

        let png = create_test_png(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4]);

        let key1 = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 1,
            strike_ppem: 16,
        };
        let key2 = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 2,
            strike_ppem: 16,
        };
        let key3 = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 3,
            strike_ppem: 16,
        };

        let _ = cache.get_or_decode(key1, &png).unwrap();
        assert_eq!(cache.current_bytes(), 4);
        assert_eq!(cache.len(), 1);

        let _ = cache.get_or_decode(key2, &png).unwrap();
        assert_eq!(cache.current_bytes(), 8);
        assert_eq!(cache.len(), 2);

        // Inserting key3 would make total bytes 12 > 10. key1 is evicted.
        let _ = cache.get_or_decode(key3, &png).unwrap();
        assert_eq!(cache.current_bytes(), 8);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_image_larger_than_max_bytes_not_cached() {
        // Cache max bytes = 2. A 1x1 RGBA image is 4 bytes.
        let mut cache = BitmapGlyphCache::with_bounds(10, 2);
        let png = create_test_png(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4]);
        let key = BitmapCacheKey {
            font_key: 1,
            face_index: 0,
            glyph_id: 1,
            strike_ppem: 16,
        };

        let img = cache.get_or_decode(key, &png);
        assert!(img.is_some());
        // Should not be stored in cache
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.current_bytes(), 0);
    }
}
