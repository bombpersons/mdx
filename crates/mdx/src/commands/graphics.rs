//! Export mode-5 graphics from verified buffers. No cartridge-specific names
//! or ROM source locations are inferred from the current contents of VRAM.
use super::render::{Artifact, VerifiedSnapshot};
use crate::envelope::{Coverage, Envelope, Failure};
use mdcore::vdp::{Framebuffer, Vdp};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const TOOL: &str = "mdx.graphics";

#[derive(Serialize, schemars::JsonSchema)]
pub struct GraphicsData {
    memory_artifacts: BTreeMap<String, Artifact>,
    tile_count: usize,
    nonempty_tile_count: usize,
    tile_indices: Artifact,
    tile_sheets: Vec<Artifact>,
    layout: Artifact,
    transparent_index: u8,
    sheet_columns: usize,
    palette_rgb: Vec<(u8, u8, u8)>,
    planes: Vec<PlaneSummary>,
    sprite_count: usize,
    linked_sprites: Vec<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sprite_groups: Vec<SpriteGroupImage>,
}

/// Caller-selected hardware sprites to compose into one image. Grouping is
/// never inferred from neighbouring table entries.
#[derive(Debug, Clone, serde::Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpriteGroup {
    /// File-name label: letters, digits, '-' and '_' (1..32).
    pub name: String,
    /// Sprite table indices; earlier entries are drawn in front.
    #[schemars(length(min = 1, max = 80))]
    pub sprites: Vec<usize>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SpriteGroupImage {
    name: String,
    sprites: Vec<usize>,
    /// Union bounds in screen coordinates (sprite x/y minus 128).
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    /// Transparent RGBA PNG: pattern index 0 is transparent; sprites keep
    /// their own palette and flips.
    image: Artifact,
}

#[derive(Serialize, schemars::JsonSchema)]
struct Cell {
    vram_address: usize,
    raw: u16,
    tile: u16,
    palette: u8,
    horizontal_flip: bool,
    vertical_flip: bool,
    high_priority: bool,
}

#[derive(Serialize, schemars::JsonSchema)]
struct PlaneSummary {
    name: &'static str,
    base: usize,
    columns: usize,
    rows: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct Plane {
    summary: PlaneSummary,
    cells: Vec<Cell>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct Sprite {
    index: usize,
    x: i32,
    y: i32,
    columns: usize,
    rows: usize,
    link: usize,
    attributes: Cell,
    /// Column-major hardware pattern order, before flips.
    tiles: Vec<u16>,
}

fn failure(code: &str, message: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        message: message.into(),
        remediation: None,
    }
}

fn word(v: &Vdp, address: usize) -> u16 {
    let address = address & 0xFFFE;
    u16::from_be_bytes([v.vram[address], v.vram[address + 1]])
}

fn cell(v: &Vdp, address: usize) -> Cell {
    let raw = word(v, address);
    Cell {
        vram_address: address & 0xFFFE,
        raw,
        tile: raw & 0x07FF,
        palette: ((raw >> 13) & 3) as u8,
        horizontal_flip: raw & 0x0800 != 0,
        vertical_flip: raw & 0x1000 != 0,
        high_priority: raw & 0x8000 != 0,
    }
}

fn artifact(output: &Path, name: &str, extension: &str, bytes: &[u8]) -> Result<Artifact, Failure> {
    crate::artifacts::named(output, name, extension, bytes)
}

pub fn run(
    path: &Path,
    output: Option<&Path>,
    store: &Path,
    groups: &[SpriteGroup],
) -> Result<Envelope<GraphicsData>, Failure> {
    if groups.len() > 16
        || groups.iter().any(|g| {
            g.name.is_empty()
                || g.name.len() > 32
                || !g.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || g.sprites.is_empty()
                || g.sprites.len() > 80
        })
    {
        return Err(failure(
            "invalid_arguments",
            "sprite_groups: at most 16 groups, names of 1..32 letters/digits/-/_, 1..80 sprites each",
        ));
    }
    let VerifiedSnapshot {
        vdp: v,
        memory_artifacts,
        subject,
    } = super::render::load(path, store)?;
    if v.registers[1] & 4 == 0 || v.registers[12] & 6 != 0 {
        return Err(failure(
            "graphics_mode_unsupported",
            "requires non-interlaced mode 5",
        ));
    }
    let dimensions = |bits: u8| match bits & 3 {
        0 => Some(32),
        1 => Some(64),
        3 => Some(128),
        _ => None,
    };
    let columns = dimensions(v.registers[16])
        .ok_or_else(|| failure("graphics_mode_unsupported", "invalid plane width"))?;
    let rows = dimensions(v.registers[16] >> 4)
        .ok_or_else(|| failure("graphics_mode_unsupported", "invalid plane height"))?;
    let wide = v.registers[12] & 1 != 0;
    let summaries = vec![
        PlaneSummary {
            name: "a",
            base: usize::from(v.registers[2] & 0x38) << 10,
            columns,
            rows,
        },
        PlaneSummary {
            name: "b",
            base: usize::from(v.registers[4] & 7) << 13,
            columns,
            rows,
        },
        PlaneSummary {
            name: "window",
            base: usize::from(v.registers[3] & if wide { 0x3C } else { 0x3E }) << 10,
            columns: if wide { 64 } else { 32 },
            rows: 32,
        },
    ];
    let planes: Vec<_> = summaries
        .into_iter()
        .map(|summary| {
            let cells = (0..summary.columns * summary.rows)
                .map(|i| cell(&v, summary.base + i * 2))
                .collect();
            Plane { summary, cells }
        })
        .collect();
    let sprite_count = if wide { 80 } else { 64 };
    let sprite_base = usize::from(v.registers[5] & if wide { 0x7E } else { 0x7F }) << 9;
    let sprites: Vec<_> = (0..sprite_count)
        .map(|index| {
            let address = sprite_base + index * 8;
            let size_link = word(&v, address + 2);
            let columns = usize::from((size_link >> 10) & 3) + 1;
            let rows = usize::from((size_link >> 8) & 3) + 1;
            let attributes = cell(&v, address + 4);
            let tiles = (0..columns * rows)
                .map(|i| (attributes.tile + i as u16) & 0x07FF)
                .collect();
            Sprite {
                index,
                x: i32::from(word(&v, address + 6) & 0x01FF) - 128,
                y: i32::from(word(&v, address) & 0x01FF) - 128,
                columns,
                rows,
                link: usize::from(size_link & 0x7F),
                attributes,
                tiles,
            }
        })
        .collect();
    let mut linked_sprites = vec![0];
    let mut limitations = vec![
        "snapshot graphics only; cartridge source addresses and game meanings are not inferred".into(),
        "maps are complete row-major VRAM tables, not a world-map or visible-cell inventory".into(),
        "sprite links do not model per-scanline limits, masking, or raster changes".into(),
        "tile sheets show index zero as an opaque palette swatch; native compositing must treat it as transparent".into(),
    ];
    loop {
        let next = sprites[*linked_sprites.last().unwrap()].link;
        if next == 0 {
            break;
        }
        if next >= sprite_count || linked_sprites.contains(&next) {
            limitations.push(format!(
                "sprite chain stops at invalid or repeated link {next}; full table is exported"
            ));
            break;
        }
        linked_sprites.push(next);
    }
    let default_output = PathBuf::from(".mdx/graphics").join(&subject.sha256[..16]);
    let output = output.unwrap_or(&default_output);
    fs::create_dir_all(output).map_err(|e| failure("graphics_write_failed", e.to_string()))?;
    // Four high-nibble-first packed bytes per row, eight rows per pattern.
    let indices: Vec<u8> = v
        .vram
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 15])
        .collect();
    let nonempty_tile_count = indices
        .chunks_exact(64)
        .filter(|tile| tile.iter().any(|i| *i != 0))
        .count();
    let tile_indices = artifact(output, "tiles", "indices.bin", &indices)?;
    let mut tile_sheets = Vec::new();
    for palette in 0..4 {
        let mut frame = Framebuffer::new(256, 512);
        for (tile, texels) in indices.chunks_exact(64).enumerate() {
            for (i, texel) in texels.iter().enumerate() {
                frame.set(
                    tile % 32 * 8 + i % 8,
                    tile / 32 * 8 + i / 8,
                    v.palette(palette * 16 + usize::from(*texel)),
                );
            }
        }
        let bytes = super::boot::encode_png(&frame)
            .map_err(|e| failure("graphics_write_failed", e.to_string()))?;
        tile_sheets.push(artifact(
            output,
            &format!("palette-{palette}"),
            "png",
            &bytes,
        )?);
    }
    let mut sprite_groups = Vec::new();
    for group in groups {
        if let Some(bad) = group.sprites.iter().find(|i| **i >= sprite_count) {
            return Err(failure("invalid_arguments", format!("sprite index {bad} is outside the {sprite_count}-entry table")));
        }
        let members: Vec<&Sprite> = group.sprites.iter().map(|i| &sprites[*i]).collect();
        let left = members.iter().map(|s| s.x).min().unwrap();
        let top = members.iter().map(|s| s.y).min().unwrap();
        let right = members.iter().map(|s| s.x + s.columns as i32 * 8).max().unwrap();
        let bottom = members.iter().map(|s| s.y + s.rows as i32 * 8).max().unwrap();
        let (width, height) = ((right - left) as usize, (bottom - top) as usize);
        let mut rgba = vec![0u8; width * height * 4];
        // Draw back to front so the first listed sprite ends up in front.
        for sprite in members.iter().rev() {
            let a = &sprite.attributes;
            for column in 0..sprite.columns {
                for row in 0..sprite.rows {
                    let tile = usize::from((a.tile + (column * sprite.rows + row) as u16) & 0x07FF);
                    let dx = if a.horizontal_flip { sprite.columns - 1 - column } else { column };
                    let dy = if a.vertical_flip { sprite.rows - 1 - row } else { row };
                    for ty in 0..8 {
                        for tx in 0..8 {
                            let sx = if a.horizontal_flip { 7 - tx } else { tx };
                            let sy = if a.vertical_flip { 7 - ty } else { ty };
                            let texel = indices[tile * 64 + sy * 8 + sx];
                            if texel == 0 {
                                continue;
                            }
                            let (r, g, b) = v.palette(usize::from(a.palette) * 16 + usize::from(texel));
                            let px = (sprite.x - left) as usize + dx * 8 + tx;
                            let py = (sprite.y - top) as usize + dy * 8 + ty;
                            rgba[(py * width + px) * 4..][..4].copy_from_slice(&[r, g, b, 255]);
                        }
                    }
                }
            }
        }
        let bytes = encode_rgba_png(&rgba, width, height)
            .map_err(|e| failure("graphics_write_failed", e.to_string()))?;
        sprite_groups.push(SpriteGroupImage {
            name: group.name.clone(),
            sprites: group.sprites.clone(),
            x: left,
            y: top,
            width,
            height,
            image: artifact(output, &format!("sprites-{}", group.name), "png", &bytes)?,
        });
    }
    let layout = serde_json::json!({"schema":"mdx.graphics.layout.v1", "registers":v.registers.to_vec(),
        "planes":planes, "sprite_table_base":sprite_base, "sprites":sprites,
        "linked_sprites":linked_sprites, "horizontal_scroll_base":usize::from(v.registers[13] & 0x3F) << 10,
        "vsram":v.vsram, "tile_index_layout":"2048 tiles * 8 rows * 8 columns; unpacked u8 indices; column-major sprite tile order",
        "limitations":limitations});
    let layout = artifact(
        output,
        "layout",
        "json",
        &serde_json::to_vec(&layout).expect("serializable layout"),
    )?;
    let coverage = Coverage { limitations, ..Coverage::complete(vec![
        "all 2048 mode-5 patterns, all four palettes, all three name tables, complete sprite table".into()]) };
    Ok(Envelope::ok(
        TOOL,
        Some(subject),
        coverage,
        GraphicsData {
            memory_artifacts,
            tile_count: 2048,
            nonempty_tile_count,
            tile_indices,
            tile_sheets,
            layout,
            transparent_index: 0,
            sheet_columns: 32,
            palette_rgb: (0..64).map(|i| v.palette(i)).collect(),
            planes: planes.into_iter().map(|p| p.summary).collect(),
            sprite_count,
            linked_sprites,
            sprite_groups,
        },
    ))
}

fn encode_rgba_png(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, png::EncodingError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Best);
        encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        encoder.write_header()?.write_image_data(rgba)?;
    }
    Ok(bytes)
}
