//! Bounded queries of captured memory, without re-executing the cartridge.
use crate::{
    artifacts::{self, Artifact},
    context::Context,
    envelope::{Coverage, Envelope, Failure, Subject},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Memory {
    Wram,
    Vram,
    Cram,
    Vsram,
    Z80Ram,
}
impl Memory {
    fn name(self) -> &'static str {
        match self {
            Self::Wram => "wram",
            Self::Vram => "vram",
            Self::Cram => "cram",
            Self::Vsram => "vsram",
            Self::Z80Ram => "z80_ram",
        }
    }
    fn len(self) -> usize {
        match self {
            Self::Wram | Self::Vram => 65536,
            Self::Cram => 128,
            Self::Vsram => 80,
            Self::Z80Ram => 8192,
        }
    }
}
fn length() -> usize {
    256
}
fn limit() -> usize {
    64
}
fn alignment() -> usize {
    1
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {
    pub snapshot: String,
    pub memory: Memory,
    /// WRAM accepts folded offsets or full CPU aliases. Other buffers use offsets.
    #[serde(default)]
    pub address: u32,
    #[serde(default = "length")]
    #[schemars(range(min = 1, max = 16384))]
    pub length: usize,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    pub snapshot: String,
    pub memory: Memory,
    /// Exact hexadecimal byte pattern, with optional ASCII whitespace.
    pub pattern: String,
    #[serde(default = "alignment")]
    #[schemars(range(min = 1, max = 65536))]
    pub alignment: usize,
    /// Zero-based match index for pagination, not a memory address.
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "limit")]
    #[schemars(range(min = 1, max = 4096))]
    pub limit: usize,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffArgs {
    pub left: String,
    pub right: String,
    pub memory: Memory,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "limit")]
    #[schemars(range(min = 1, max = 1024))]
    pub limit: usize,
}

#[derive(Serialize, JsonSchema)]
pub struct ReadData {
    pub artifact: Artifact,
    pub memory: Memory,
    pub address: u32,
    pub folded_offset: usize,
    pub bytes: String,
    pub length: usize,
}

#[derive(Serialize, JsonSchema)]
pub struct SearchData {
    pub artifact: Artifact,
    pub memory: Memory,
    pub pattern: String,
    pub matches_total: usize,
    pub offset: usize,
    pub matches: Vec<usize>,
    pub next_offset: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct Change {
    pub offset: usize,
    pub before: String,
    pub after: String,
}

#[derive(Serialize, JsonSchema)]
pub struct DiffData {
    pub left: Artifact,
    pub right: Artifact,
    pub memory: Memory,
    pub changed_bytes: usize,
    pub ranges_total: usize,
    pub offset: usize,
    pub ranges: Vec<Change>,
    pub next_offset: Option<usize>,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn decode_hex(text: &str) -> Result<Vec<u8>, Failure> {
    let clean: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if clean.is_empty() || clean.len() > 32768 || clean.len() % 2 != 0 || !clean.is_ascii() {
        return Err(artifacts::failure(
            "invalid_arguments",
            "hex pattern must contain 1..16384 complete bytes",
        ));
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&clean[i..i + 2], 16)
                .map_err(|_| artifacts::failure("invalid_arguments", "invalid hexadecimal byte"))
        })
        .collect()
}

fn load(
    source: &str,
    memory: Memory,
    context: &Context,
) -> Result<(Subject, Artifact, Vec<u8>), Failure> {
    let path = artifacts::resolve(&context.store, source)?;
    let raw = artifacts::read_bounded(&path, 64 * 1024 * 1024)?;
    let value: Value = serde_json::from_slice(&raw)
        .map_err(|e| artifacts::failure("snapshot_invalid", e.to_string()))?;
    let body = value.get("data").unwrap_or(&value);
    let entries: BTreeMap<String, Artifact> =
        serde_json::from_value(body["memory_artifacts"].clone())
            .map_err(|e| artifacts::failure("snapshot_invalid", e.to_string()))?;
    let mut artifact = entries.get(memory.name()).cloned().ok_or_else(|| {
        artifacts::failure("snapshot_invalid", "selected memory buffer is missing")
    })?;
    artifacts::rebase(&mut artifact, &context.store)?;
    let bytes = artifacts::verified(&artifact)?;
    if bytes.len() != memory.len() {
        return Err(artifacts::failure(
            "artifact_size_mismatch",
            "selected buffer has the wrong hardware size",
        ));
    }
    artifact.path = artifacts::absolute(&artifact.path)?;
    artifact.id = format!("sha256:{}", artifact.sha256);
    Ok((
        Subject {
            path: path.display().to_string(),
            sha256: artifacts::digest(&raw),
            bytes: raw.len(),
            format: "memory-snapshot-json".into(),
            architecture: None,
        },
        artifact,
        bytes,
    ))
}

pub fn read(args: ReadArgs, context: &Context) -> Result<Envelope<ReadData>, Failure> {
    if args.length == 0 || args.length > 16384 {
        return Err(artifacts::failure(
            "limit_exceeded",
            "memory read pages must be 1..16384 bytes",
        ));
    }
    let (subject, artifact, bytes) = load(&args.snapshot, args.memory, context)?;
    let start = if matches!(args.memory, Memory::Wram) {
        if args.address > 65535
            && mdcore::memmap::region(args.address & 0xFF_FFFF) != mdcore::Region::Wram
        {
            return Err(artifacts::failure(
                "invalid_arguments",
                "address is not a work-RAM alias",
            ));
        }
        mdcore::memmap::fold_to_wram(args.address) as usize
    } else {
        args.address as usize
    };
    if start >= bytes.len() || args.length > bytes.len() {
        return Err(artifacts::failure(
            "invalid_arguments",
            "memory address or range is outside the selected buffer",
        ));
    }
    let selected: Vec<_> = (0..args.length)
        .map(|i| bytes[(start + i) % bytes.len()])
        .collect();
    Ok(Envelope::ok(
        "mdx.memory.read",
        Some(subject),
        Coverage::complete(vec![format!(
            "{} byte read at {} offset {start}; wraps at buffer end",
            args.length,
            args.memory.name()
        )]),
        ReadData {
            artifact,
            memory: args.memory,
            address: args.address,
            folded_offset: start,
            bytes: hex(&selected),
            length: selected.len(),
        },
    ))
}

pub fn search(args: SearchArgs, context: &Context) -> Result<Envelope<SearchData>, Failure> {
    if args.limit == 0 || args.limit > 4096 || args.alignment == 0 || args.alignment > 65536 {
        return Err(artifacts::failure(
            "limit_exceeded",
            "matches <= 4096; alignment must be 1..65536",
        ));
    }
    let pattern = decode_hex(&args.pattern)?;
    let (subject, artifact, bytes) = load(&args.snapshot, args.memory, context)?;
    let mut total = 0;
    let mut matches = Vec::new();
    if pattern.len() <= bytes.len() {
        for i in (0..=bytes.len() - pattern.len()).step_by(args.alignment) {
            if bytes[i..i + pattern.len()] == pattern {
                if total >= args.offset && matches.len() < args.limit {
                    matches.push(i);
                }
                total += 1;
            }
        }
    }
    if args.offset > total {
        return Err(artifacts::failure(
            "invalid_arguments",
            "match offset is beyond result end",
        ));
    }
    let next = args.offset + matches.len();
    Ok(Envelope::ok(
        "mdx.memory.search",
        Some(subject),
        Coverage::complete(vec![format!(
            "searched {} bytes; page {}..{next} of {total} matches; patterns do not wrap",
            bytes.len(),
            args.offset
        )]),
        SearchData {
            artifact,
            memory: args.memory,
            pattern: hex(&pattern),
            matches_total: total,
            offset: args.offset,
            matches,
            next_offset: (next < total).then_some(next),
        },
    ))
}

pub fn diff(args: DiffArgs, context: &Context) -> Result<Envelope<DiffData>, Failure> {
    if args.limit == 0 || args.limit > 1024 {
        return Err(artifacts::failure(
            "limit_exceeded",
            "diff pages must contain 1..1024 ranges",
        ));
    }
    let (_, left, a) = load(&args.left, args.memory, context)?;
    let (_, right, b) = load(&args.right, args.memory, context)?;
    let mut i = 0;
    let mut changed = 0;
    let mut total = 0;
    let mut ranges = Vec::new();
    while i < a.len() {
        if a[i] == b[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < a.len() && a[i] != b[i] && i - start < 32 {
            changed += 1;
            i += 1;
        }
        if total >= args.offset && ranges.len() < args.limit {
            ranges.push(Change {
                offset: start,
                before: hex(&a[start..i]),
                after: hex(&b[start..i]),
            });
        }
        total += 1;
    }
    if args.offset > total {
        return Err(artifacts::failure(
            "invalid_arguments",
            "diff offset is beyond result end",
        ));
    }
    let next = args.offset + ranges.len();
    let mut coverage = Coverage::complete(vec![format!(
        "all {} bytes compared; page {}..{next} of {total} changed ranges",
        a.len(),
        args.offset
    )]);
    coverage.limitations.push("final-buffer comparison only; no execution alignment or writer/causality inference; ranges split at 32 bytes".into());
    Ok(Envelope::ok(
        "mdx.memory.diff",
        None,
        coverage,
        DiffData {
            left,
            right,
            memory: args.memory,
            changed_bytes: changed,
            ranges_total: total,
            offset: args.offset,
            ranges,
            next_offset: (next < total).then_some(next),
        },
    ))
}
