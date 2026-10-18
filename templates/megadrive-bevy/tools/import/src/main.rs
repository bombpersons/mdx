//! Offline importers: data read from the ROM -> engine-native assets.
//!
//! The ROM's raw formats (tile bytes, colour words, compressed blocks, level
//! tables) are decoded here and nowhere else; the game and engine only see
//! the results: indexed PNGs, RON tables and palettes, WAV. Each output gets
//! a `<output>.provenance.json` beside it naming the ROM bytes it came from.
//!
//! Find the offsets with mdx (`disasm`, `watch` on the VDP port, `graphics`),
//! record them in `docs/evidence.md`, then add one subcommand per format.
//! Most games compress their art (Nemesis, Kosinski, Enigma or their own
//! schemes): add the decompressor here, verified against what mdx shows in
//! VRAM after the game loads it.
//!
//! cargo run -p import -- tiles --rom game.md --offset 0x1A000 --count 64 --out game/assets/hud/tiles.png
//! cargo run -p import -- palette --rom game.md --offset 0x2400 --lines 4 --out game/assets/hud/palette.ron
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  import tiles   --rom PATH --offset HEX --count N [--columns N] --out FILE.png
  import palette --rom PATH --offset HEX [--lines N] --out FILE.ron";

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("import: {error}\n{USAGE}");
        std::process::exit(2);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let command = args.first().ok_or("no command")?.as_str();
    let value = |name: &str| -> Result<&str, String> {
        let at = args.iter().position(|a| a == name).ok_or(format!("{name} is required"))?;
        args.get(at + 1).map(String::as_str).ok_or(format!("{name} needs a value"))
    };
    let number = |name: &str, default: Option<usize>| -> Result<usize, String> {
        match (value(name), default) {
            (Err(_), Some(default)) => Ok(default),
            (text, _) => parse_number(text?).ok_or(format!("{name} is not a number")),
        }
    };
    let rom = Source::read(Path::new(value("--rom")?))?;
    let out = PathBuf::from(value("--out")?);
    let offset = number("--offset", None)?;
    match command {
        "tiles" => {
            let count = number("--count", None)?;
            let columns = number("--columns", Some(16))?.clamp(1, count.max(1));
            let bytes = rom.slice(offset, count * 32)?;
            write_png(&out, &tiles_to_indexed(bytes, columns))?;
            rom.provenance(&out, offset, bytes.len(), "4bpp tiles, 32 bytes each")
        }
        "palette" => {
            let lines = number("--lines", Some(1))?;
            let bytes = rom.slice(offset, lines * 32)?;
            std::fs::write(&out, palettes_to_ron(bytes)).map_err(|e| format!("{}: {e}", out.display()))?;
            rom.provenance(&out, offset, bytes.len(), "colour words 0000BBB0GGG0RRR0, 16 per line")
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

fn parse_number(text: &str) -> Option<usize> {
    match text.strip_prefix("0x") {
        Some(hex) => usize::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// The ROM image, with what identifies it.
struct Source {
    path: PathBuf,
    bytes: Vec<u8>,
}

impl Source {
    fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self { path: path.to_path_buf(), bytes })
    }

    fn slice(&self, offset: usize, length: usize) -> Result<&[u8], String> {
        self.bytes
            .get(offset..offset + length)
            .ok_or(format!("{length} bytes at {offset:#X} run past the ROM's {} bytes", self.bytes.len()))
    }

    /// Writes `<out>.provenance.json`: the ROM (name, size, FNV-1a 64 hash),
    /// the byte range read and what it was decoded as.
    fn provenance(&self, out: &Path, offset: usize, length: usize, format: &str) -> Result<(), String> {
        let name = self.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let text = format!(
            "{{\n  \"rom\": {name:?},\n  \"rom_bytes\": {},\n  \"rom_fnv1a64\": \"{:016x}\",\n  \"offset\": \"{offset:#X}\",\n  \"length\": {length},\n  \"format\": {format:?}\n}}\n",
            self.bytes.len(),
            fnv1a64(&self.bytes),
        );
        let path = PathBuf::from(format!("{}.provenance.json", out.display()));
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, &b| (hash ^ b as u64).wrapping_mul(0x100000001b3))
}

/// An image of palette indices, one per pixel.
struct Indexed {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Mega Drive tiles: 8x8 pixels, 4 bits each, two pixels per byte with the
/// left one in the high nibble, rows top to bottom. Laid out `columns` tiles
/// per row.
fn tiles_to_indexed(bytes: &[u8], columns: usize) -> Indexed {
    let count = bytes.len() / 32;
    let rows = count.div_ceil(columns);
    let (width, height) = (columns * 8, rows * 8);
    let mut pixels = vec![0; width * height];
    for (tile, data) in bytes.chunks_exact(32).enumerate() {
        let (tx, ty) = ((tile % columns) * 8, (tile / columns) * 8);
        for (i, byte) in data.iter().enumerate() {
            let (x, y) = (tx + (i % 4) * 2, ty + i / 4);
            pixels[y * width + x] = byte >> 4;
            pixels[y * width + x + 1] = byte & 15;
        }
    }
    Indexed { width: width as u32, height: height as u32, pixels }
}

/// A 4-bit indexed PNG with a grey preview palette (the game draws it with
/// its own palettes; only the indices matter).
fn write_png(path: &Path, image: &Indexed) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Four);
    encoder.set_palette((0..16u8).flat_map(|i| [i * 17; 3]).collect::<Vec<_>>());
    encoder.set_trns(vec![0u8]);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let packed: Vec<u8> = image
        .pixels
        .chunks(image.width as usize)
        .flat_map(|row| row.chunks(2).map(|p| p[0] << 4 | p.get(1).copied().unwrap_or(0)).collect::<Vec<_>>())
        .collect();
    writer.write_image_data(&packed).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

/// Colour words (big-endian `0000BBB0GGG0RRR0`) as palette lines in the
/// engine's RON form: `[[(r, g, b), ...16], ...]`, levels 0..=7.
fn palettes_to_ron(bytes: &[u8]) -> String {
    let lines: Vec<String> = bytes
        .chunks_exact(32)
        .map(|line| {
            let colours: Vec<String> = line
                .chunks_exact(2)
                .map(|w| {
                    let word = u16::from_be_bytes([w[0], w[1]]);
                    format!("({}, {}, {})", (word >> 1) & 7, (word >> 5) & 7, (word >> 9) & 7)
                })
                .collect();
            format!("    [{}],\n", colours.join(", "))
        })
        .collect();
    format!("[\n{}]\n", lines.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_unpack_high_nibble_first() {
        let mut tile = [0u8; 32];
        tile[0] = 0x12; // pixels (0,0)=1, (1,0)=2
        tile[31] = 0x0F; // pixel (7,7)=15
        let image = tiles_to_indexed(&tile, 1);
        assert_eq!((image.width, image.height), (8, 8));
        assert_eq!(&image.pixels[..2], &[1, 2]);
        assert_eq!(image.pixels[63], 15);
    }

    #[test]
    fn colour_words_become_levels() {
        // Blue 7, green 2, red 5: 0000 1110 0100 1010.
        let mut line = vec![0u8; 32];
        line[2..4].copy_from_slice(&0x0E4Au16.to_be_bytes());
        let ron = palettes_to_ron(&line);
        assert!(ron.starts_with("[\n    [(0, 0, 0), (5, 2, 7),"), "{ron}");
    }
}
