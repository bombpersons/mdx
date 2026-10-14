//! Pixel comparison of two PNGs, e.g. an mdx screenshot against a native
//! port's `game_screenshot`. Counts are absolute; a diff PNG shows changed
//! pixels in magenta over a dimmed copy of the left image.
use crate::artifacts::{self, Artifact};
use crate::context::Context;
use crate::envelope::{Coverage, Envelope, Failure, Subject};
use serde::{Deserialize, Serialize};

pub const TOOL: &str = "mdx.image.compare";

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareArgs {
    /// PNG path (relative to the project) or artifact ID.
    pub left: String,
    pub right: String,
    /// Named rectangles to count separately (e.g. "header", "playfield").
    #[serde(default)]
    #[schemars(length(max = 64))]
    pub regions: Vec<Region>,
    /// Output DIRECTORY for a digest-named diff PNG; the store ID always works.
    #[serde(default)]
    #[schemars(with = "String")]
    pub out: Option<std::path::PathBuf>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RegionResult {
    pub name: String,
    pub pixels: u64,
    pub changed_pixels: u64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CompareData {
    pub width: u32,
    pub height: u32,
    pub pixels: u64,
    pub changed_pixels: u64,
    /// Smallest rectangle containing every changed pixel, if any: [x, y, w, h].
    pub changed_bounds: Option<[u32; 4]>,
    pub regions: Vec<RegionResult>,
    pub left_sha256: String,
    pub right_sha256: String,
    /// Magenta = changed; unchanged pixels are the left image dimmed.
    pub diff_artifact: Artifact,
}

/// Decode a PNG to RGB, expanding palettes and dropping alpha (composited on black).
fn rgb(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), Failure> {
    let invalid = |e: png::DecodingError| artifacts::failure("image_invalid", e.to_string());
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(invalid)?;
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).map_err(invalid)?;
    data.truncate(info.buffer_size());
    let (color, _) = reader.output_color_type();
    let channels = color.samples();
    let pixels = data
        .chunks_exact(channels)
        .flat_map(|p| match channels {
            1 => [p[0], p[0], p[0]],
            2 => {
                let v = (u16::from(p[0]) * u16::from(p[1]) / 255) as u8;
                [v, v, v]
            }
            3 => [p[0], p[1], p[2]],
            _ => [p[0], p[1], p[2]].map(|c| (u16::from(c) * u16::from(p[3]) / 255) as u8),
        })
        .collect();
    Ok((info.width, info.height, pixels))
}

pub fn compare(args: CompareArgs, context: &Context) -> Result<Envelope<CompareData>, Failure> {
    let load = |source: &str| -> Result<(Vec<u8>, std::path::PathBuf), Failure> {
        let path = artifacts::resolve(&context.store, source)?;
        Ok((artifacts::read_bounded(&path, 64 * 1024 * 1024)?, path))
    };
    let (left_bytes, left_path) = load(&args.left)?;
    let (right_bytes, _) = load(&args.right)?;
    let (width, height, a) = rgb(&left_bytes)?;
    let (rw, rh, b) = rgb(&right_bytes)?;
    if (width, height) != (rw, rh) {
        return Err(artifacts::failure(
            "image_size_mismatch",
            format!("left is {width}x{height}, right is {rw}x{rh}"),
        ));
    }
    if let Some(r) = args.regions.iter().find(|r| {
        r.width == 0 || r.height == 0 || r.x + r.width > width || r.y + r.height > height
    }) {
        return Err(artifacts::failure(
            "invalid_arguments",
            format!("region {:?} must be non-empty and inside {width}x{height}", r.name),
        ));
    }
    let mut changed = vec![false; (width * height) as usize];
    let mut diff = Vec::with_capacity(a.len());
    let mut bounds: Option<[u32; 4]> = None;
    for (i, (pa, pb)) in a.chunks_exact(3).zip(b.chunks_exact(3)).enumerate() {
        if pa != pb {
            changed[i] = true;
            diff.extend([255, 0, 255]);
            let (x, y) = (i as u32 % width, i as u32 / width);
            bounds = Some(match bounds {
                None => [x, y, x, y],
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            });
        } else {
            diff.extend(pa.iter().map(|c| c / 3));
        }
    }
    let changed_pixels = changed.iter().filter(|c| **c).count() as u64;
    let regions = args
        .regions
        .iter()
        .map(|r| RegionResult {
            name: r.name.clone(),
            pixels: u64::from(r.width) * u64::from(r.height),
            changed_pixels: (r.y..r.y + r.height)
                .flat_map(|y| (r.x..r.x + r.width).map(move |x| (y * width + x) as usize))
                .filter(|i| changed[*i])
                .count() as u64,
        })
        .collect();
    let mut frame = mdcore::vdp::Framebuffer::new(width as usize, height as usize);
    frame.data = diff;
    let png = super::boot::encode_png(&frame).map_err(|e| artifacts::failure("image_write_failed", e.to_string()))?;
    let diff_artifact = match &args.out {
        Some(out) => artifacts::named(out, "image-diff", "png", &png)?,
        None => artifacts::put(&context.store, &png)?,
    };
    let digest = |bytes: &[u8]| {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    };
    let mut coverage = Coverage::complete(vec![format!(
        "all {} pixels compared exactly as decoded RGB; alpha composited on black",
        u64::from(width) * u64::from(height)
    )]);
    Ok(Envelope::ok(
        TOOL,
        Some(Subject {
            path: left_path.display().to_string(),
            sha256: digest(&left_bytes),
            bytes: left_bytes.len(),
            format: "png".into(),
            architecture: None,
        }),
        coverage,
        CompareData {
            width,
            height,
            pixels: u64::from(width) * u64::from(height),
            changed_pixels,
            changed_bounds: bounds.map(|[x0, y0, x1, y1]| [x0, y0, x1 - x0 + 1, y1 - y0 + 1]),
            regions,
            left_sha256: digest(&left_bytes),
            right_sha256: digest(&right_bytes),
            diff_artifact,
        },
    ))
}
