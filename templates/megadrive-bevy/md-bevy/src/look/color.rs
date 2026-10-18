//! Mega Drive colours and palettes.
//!
//! The console draws with 9-bit colours (three bits per channel) picked from
//! four 16-colour palette lines; colour 0 of every line is transparent. Art
//! is authored against those lines, and palette changes (fades, cycling,
//! flashes) are how most games animate colour, so the look keeps them as
//! data: an indexed image plus the palette it is drawn with.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// One colour: red, green and blue levels, each 0..=7.
///
/// RON form: `(r, g, b)`, for example `(7, 4, 0)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "(u8, u8, u8)", into = "(u8, u8, u8)")]
pub struct MdColor {
    r: u8,
    g: u8,
    b: u8,
}

/// The brightness shadow/highlight mode applies to a pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shade {
    #[default]
    Normal,
    Shadow,
    Highlight,
}

impl MdColor {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };
    pub const WHITE: Self = Self { r: 7, g: 7, b: 7 };

    /// Panics if a level is above 7.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        assert!(r < 8 && g < 8 && b < 8, "Mega Drive colour levels are 0..=7");
        Self { r, g, b }
    }

    pub const fn levels(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }

    /// 8-bit sRGB, with the same level mapping as mdx's (and Genesis Plus
    /// GX's) renderer, so a native screenshot compares exactly with mdx's.
    pub const fn to_srgb_u8(self, shade: Shade) -> [u8; 3] {
        // Shadow is level c, normal 2c and highlight c + 7 on a 0..=14 scale,
        // expanded to RGB565 and then to 8 bits.
        const fn level(c: u8, shade: Shade) -> u32 {
            match shade {
                Shade::Shadow => c as u32,
                Shade::Normal => (c as u32) << 1,
                Shade::Highlight => c as u32 + 7,
            }
        }
        let (r, g, b) = (level(self.r, shade), level(self.g, shade), level(self.b, shade));
        [
            (((r << 1) | (r >> 3)) * 255 / 31) as u8,
            (((g << 2) | (g >> 2)) * 255 / 63) as u8,
            (((b << 1) | (b >> 3)) * 255 / 31) as u8,
        ]
    }

    pub fn to_color(self) -> Color {
        let [r, g, b] = self.to_srgb_u8(Shade::Normal);
        Color::srgb_u8(r, g, b)
    }
}

impl TryFrom<(u8, u8, u8)> for MdColor {
    type Error = String;
    fn try_from((r, g, b): (u8, u8, u8)) -> Result<Self, String> {
        if r < 8 && g < 8 && b < 8 {
            Ok(Self { r, g, b })
        } else {
            Err(format!("colour ({r}, {g}, {b}): levels are 0..=7"))
        }
    }
}

impl From<MdColor> for (u8, u8, u8) {
    fn from(c: MdColor) -> Self {
        (c.r, c.g, c.b)
    }
}

/// One palette line: 16 colours, of which colour 0 is transparent in tiles
/// and sprites.
///
/// RON form: a list of 16 colours, `[(0, 0, 0), (7, 7, 7), ...]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<MdColor>", into = "Vec<MdColor>")]
pub struct Palette(pub [MdColor; 16]);

impl TryFrom<Vec<MdColor>> for Palette {
    type Error = String;
    fn try_from(colours: Vec<MdColor>) -> Result<Self, String> {
        let count = colours.len();
        colours.try_into().map(Self).map_err(|_| format!("a palette line has 16 colours, not {count}"))
    }
}

impl From<Palette> for Vec<MdColor> {
    fn from(palette: Palette) -> Self {
        palette.0.to_vec()
    }
}

/// The four palette lines the scene is drawn with, and the backdrop colour
/// shown where nothing opaque is drawn.
///
/// Changing it re-bakes every [`super::TilePlane`] chunk and
/// [`super::IndexedSprite`] that uses it, so fades and colour cycling are
/// ordinary edits of this resource. It is the art's palette, not a model of
/// colour RAM: there are no addresses or write timings here.
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
///
/// RON form: `(lines: [[...16 colours...], ...4 lines...], backdrop: (0, 0, 0))`.
pub struct Palettes {
    #[serde(with = "four_lines")]
    pub lines: [Palette; 4],
    #[serde(default)]
    pub backdrop: MdColor,
}

/// Four palette lines as a RON list rather than a tuple.
mod four_lines {
    use super::Palette;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(lines: &[Palette; 4], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(lines)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[Palette; 4], D::Error> {
        let lines = Vec::<Palette>::deserialize(d)?;
        let count = lines.len();
        lines
            .try_into()
            .map_err(|_| serde::de::Error::custom(format!("there are 4 palette lines, not {count}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_match_mdx_screenshots() {
        // Normal levels 0..=7 as mdx renders them.
        let red: Vec<u8> = (0..8).map(|c| MdColor::new(c, 0, 0).to_srgb_u8(Shade::Normal)[0]).collect();
        let green: Vec<u8> = (0..8).map(|c| MdColor::new(0, c, 0).to_srgb_u8(Shade::Normal)[1]).collect();
        assert_eq!(red, [0, 32, 65, 98, 139, 172, 205, 238]);
        assert_eq!(green, [0, 32, 68, 101, 137, 170, 206, 238]);
        assert_eq!(MdColor::WHITE.to_srgb_u8(Shade::Shadow), [115, 117, 115]);
        assert_eq!(MdColor::BLACK.to_srgb_u8(Shade::Highlight), [115, 117, 115]);
        assert_eq!(MdColor::WHITE.to_srgb_u8(Shade::Highlight), [238, 238, 238]);
    }

    #[test]
    fn colours_read_from_ron_and_reject_levels_above_seven() {
        let line: Palette = ron::from_str(&format!("[{}]", ["(7, 4, 0)"; 16].join(","))).unwrap();
        assert_eq!(line.0[3], MdColor::new(7, 4, 0));
        assert!(ron::from_str::<MdColor>("(8, 0, 0)").is_err());
        let line = format!("[{}]", ["(1, 2, 3)"; 16].join(","));
        let palettes: Palettes =
            ron::from_str(&format!("(lines: [{line}, {line}, {line}, {line}], backdrop: (0, 0, 7))")).unwrap();
        assert_eq!(palettes.lines[3].0[15], MdColor::new(1, 2, 3));
        assert_eq!(ron::from_str::<Palettes>(&ron::to_string(&palettes).unwrap()).unwrap(), palettes);
    }
}
