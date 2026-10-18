//! Indexed images: pixels that are palette indices (0..=15, 0 transparent),
//! drawn with whichever palette line the tile or sprite selects.
//!
//! On disk they are indexed (palette-mode) PNGs, which any image editor
//! opens; only the indices are used, the PNG's own palette is a preview.
//! Importers in `tools/import` write them from the ROM's tile data.
use super::color::{Palette, Shade};
use crate::assets::AssetRoot;
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

/// Width and height of one tile.
pub const TILE: u32 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    /// Row-major, one index per pixel.
    pub pixels: Vec<u8>,
}

impl IndexedImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, String> {
        if pixels.len() != (width * height) as usize {
            return Err(format!("{width}x{height} image needs {} pixels, got {}", width * height, pixels.len()));
        }
        if let Some(bad) = pixels.iter().find(|&&p| p > 15) {
            return Err(format!("palette index {bad} is above 15"));
        }
        Ok(Self { width, height, pixels })
    }

    /// Decodes an indexed or 8-bit greyscale PNG, keeping the raw indices.
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
        let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
        if !matches!(info.color_type, png::ColorType::Indexed | png::ColorType::Grayscale) {
            return Err(format!("expected an indexed PNG, got {:?}", info.color_type));
        }
        let depth = info.bit_depth as u32;
        if depth > 8 {
            return Err("expected at most 8 bits per pixel".into());
        }
        let mut pixels = Vec::with_capacity((info.width * info.height) as usize);
        for row in buffer.chunks(info.line_size).take(info.height as usize) {
            for x in 0..info.width {
                let bit = x * depth;
                let byte = row[(bit / 8) as usize];
                let shift = 8 - depth - bit % 8;
                pixels.push((byte >> shift) & ((1u16 << depth) - 1) as u8);
            }
        }
        Self::new(info.width, info.height, pixels)
    }

    /// Loads `<asset root>/<relative>` synchronously, so headless tests and
    /// the game see the same pixels.
    pub fn load(root: &AssetRoot, relative: &str) -> Result<Self, String> {
        Self::from_png(&root.read(relative)?).map_err(|e| format!("{relative}: {e}"))
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y * self.width + x) as usize]
    }

    /// How many 8x8 tiles the image holds, read left to right, top to bottom.
    pub fn tile_count(&self) -> u32 {
        (self.width / TILE) * (self.height / TILE)
    }

    /// Index of pixel (`x`, `y`) of tile `tile`.
    pub fn tile_pixel(&self, tile: u32, x: u32, y: u32) -> u8 {
        let columns = self.width / TILE;
        self.get((tile % columns) * TILE + x, (tile / columns) * TILE + y)
    }

    /// RGBA pixels drawn with `palette`; index 0 is transparent.
    pub fn bake(&self, palette: &Palette, shade: Shade) -> Image {
        let mut data = Vec::with_capacity(self.pixels.len() * 4);
        for &index in &self.pixels {
            data.extend_from_slice(&rgba(palette, index, shade));
        }
        rgba_image(self.width, self.height, data)
    }
}

/// The RGBA value of `index` in `palette`: transparent for index 0.
pub fn rgba(palette: &Palette, index: u8, shade: Shade) -> [u8; 4] {
    if index == 0 {
        return [0; 4];
    }
    let [r, g, b] = palette.0[index as usize].to_srgb_u8(shade);
    [r, g, b, 255]
}

pub(crate) fn rgba_image(width: u32, height: u32, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::color::MdColor;

    fn indexed_png(width: u32, height: u32, depth: png::BitDepth, rows: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(depth);
        encoder.set_palette(vec![0u8; 3 * 16]);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rows).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn four_bit_pngs_keep_their_indices() {
        // Two rows of four pixels: 0 1 2 3 / 15 14 13 12, packed two per byte.
        let png = indexed_png(4, 2, png::BitDepth::Four, &[0x01, 0x23, 0xFE, 0xDC]);
        let image = IndexedImage::from_png(&png).unwrap();
        assert_eq!(image.pixels, [0, 1, 2, 3, 15, 14, 13, 12]);
    }

    #[test]
    fn tiles_read_left_to_right_and_index_zero_is_transparent() {
        let mut pixels = vec![0; 16 * 8];
        pixels[8] = 5; // tile 1, pixel (0, 0)
        let image = IndexedImage::new(16, 8, pixels).unwrap();
        assert_eq!(image.tile_count(), 2);
        assert_eq!(image.tile_pixel(1, 0, 0), 5);
        let mut palette = Palette::default();
        palette.0[5] = MdColor::WHITE;
        let baked = image.bake(&palette, Shade::Normal);
        let data = baked.data.unwrap();
        assert_eq!(&data[0..4], &[0, 0, 0, 0]);
        assert_eq!(&data[32..36], &[238, 238, 238, 255]);
    }
}
