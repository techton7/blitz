mod adapter;
pub mod cache;
pub mod placement;

pub use adapter::*;
pub use cache::{BitmapCacheKey, BitmapGlyphCache};
pub use placement::compute_bitmap_placement_transform;

/// Normalized raster glyph data extracted from a font's bitmap tables (e.g. `sbix`).
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedRasterGlyph {
    /// Raw PNG bytes.
    pub data: Vec<u8>,
    /// Horizontal baseline offset.
    pub origin_x: f32,
    /// Vertical baseline offset.
    pub origin_y: f32,
    /// Image width in pixels.
    pub width: f32,
    /// Image height in pixels.
    pub height: f32,
    /// Strike pixels-per-em (ppem) of the selected bitmap image.
    pub strike_ppem: u16,
}
