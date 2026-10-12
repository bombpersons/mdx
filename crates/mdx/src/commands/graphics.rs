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
) -> Result<Envelope<GraphicsData>, Failure> {
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
        },
    ))
}
