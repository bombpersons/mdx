//! `mdx ram dump` — inspect the machine's work RAM after running the cartridge.
//!
//! Two shapes, because both come up constantly:
//!
//! * `--find VALUE` — every address whose longword equals VALUE. This is how
//!   you find where the game parked a register frame, or check whether a value
//!   it computed ever landed in RAM at all.
//! * `--range ADDR:LEN` — a hex dump, for reading a region you already know is
//!   interesting.
//!
//! Addresses are folded offsets or full 68000 work-RAM aliases. They fold into the 64 KiB work-RAM
//! aperture the same way the bus does it, so a register holding an unmasked
//! 32-bit value (0xFFFFFFFC, say) addresses the same byte it would on hardware.

use std::path::Path;

use mdcore::cartridge::Cartridge;
use mdcore::memmap::fold_to_wram;
use serde::Serialize;

use crate::envelope::{Coverage, Envelope, Failure, Subject, Warning};

pub const TOOL: &str = "mdx/ram-dump";

/// JSON range elements accept integers or explicitly hexadecimal/decimal text.
#[derive(Debug, serde::Deserialize, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum RangeValue {
    Number(u32),
    Text(String),
}

impl RangeValue {
    pub fn parse(&self) -> Result<u32, Failure> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Text(text) => {
                let text = text.trim();
                let value = if let Some(hex) =
                    text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))
                {
                    u32::from_str_radix(hex, 16)
                } else {
                    text.parse::<u32>()
                };
                value.map_err(|_| {
                    crate::artifacts::failure(
                        "invalid_arguments",
                        "RAM range elements must be u32 integers, decimal text or 0x-prefixed hex",
                    )
                })
            }
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct MatchView {
    /// The longword address, unfolded, as the CPU would name it.
    pub address: String,
    /// Offset within the 64 KiB work-RAM aperture.
    pub wram_offset: String,
    pub value: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct HexRow {
    pub address: String,
    pub wram_offset: String,
    pub bytes: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct RamData {
    pub state_artifact: Option<crate::artifacts::Artifact>,
    pub starting_frame: u64,
    pub elapsed_frames: u64,
    pub termination: String,
    pub exceptions: u64,
    pub instructions: u64,
    pub stopped_at: Option<String>,
    pub pc: String,
    pub registers: crate::commands::boot::Registers,
    pub wram_bytes_nonzero: usize,
    pub wram_bytes_total: usize,
    /// Present when `--find` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<MatchView>>,
    /// Present when `--range` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<Vec<HexRow>>,
}

pub fn run(
    rom: &Path,
    instructions: u64,
    stop_at: Option<u32>,
    find: Option<u32>,
    range: Option<(u32, u32)>,
) -> Result<(Envelope<RamData>, i32), Failure> {
    run_controlled(
        rom,
        instructions,
        stop_at,
        find,
        range,
        &crate::context::Context::default(),
    )
}

pub fn run_controlled(
    rom: &Path,
    instructions: u64,
    stop_at: Option<u32>,
    find: Option<u32>,
    range: Option<(u32, u32)>,
    context: &crate::context::Context,
) -> Result<(Envelope<RamData>, i32), Failure> {
    run_resumable(
        rom,
        instructions,
        stop_at,
        find,
        range,
        None,
        false,
        super::state::RegionChoice::Auto,
        context,
    )
}

pub fn run_resumable(
    rom: &Path,
    instructions: u64,
    stop_at: Option<u32>,
    find: Option<u32>,
    range: Option<(u32, u32)>,
    from_state: Option<&str>,
    save_state: bool,
    region: super::state::RegionChoice,
    context: &crate::context::Context,
) -> Result<(Envelope<RamData>, i32), Failure> {
    if instructions > 100_000_000 || range.is_some_and(|(_, len)| len > 65536) {
        return Err(Failure {
            code: "limit_exceeded".into(),
            message: "RAM inspection allows at most 100M steps and one 64 KiB aperture".into(),
            remediation: None,
        });
    }
    if range.is_some_and(|(addr, _)| {
        addr > 0xFFFF && mdcore::memmap::region(addr) != mdcore::Region::Wram
    }) {
        return Err(crate::artifacts::failure(
            "invalid_arguments",
            "range address must be a folded WRAM offset or a full work-RAM alias",
        ));
    }
    context.check()?;
    let cart = Cartridge::load(rom).map_err(|e| Failure {
        code: "rom_unreadable".into(),
        message: e.to_string(),
        remediation: Some("check the path points at a Mega Drive ROM image".into()),
    })?;
    let subject = Subject {
        path: cart.path.display().to_string(),
        sha256: cart.sha256.clone(),
        bytes: cart.data.len(),
        format: "mega-drive-rom".into(),
        architecture: Some("m68000".into()),
    };

    let mut system = super::state::load(cart, from_state, region, context)?;
    let starting_frame = system.vdp.frame;
    let mut executed = 0u64;
    let mut stopped_at = None;
    let mut termination = "instruction_budget";
    let mut exceptions = 0;
    while executed < instructions {
        if let Some(reason) = context.stop_reason() {
            termination = reason;
            break;
        }
        if let Some(limit) = stop_at {
            if system.cpu.pc == limit {
                stopped_at = Some(limit);
                termination = "stop_point";
                break;
            }
        }
        if system.step_instruction().1.is_some() {
            exceptions += 1;
        }
        executed += 1;
        if executed % 8192 == 0 {
            context.report(executed, instructions);
        }
    }

    let wram = &system.wram;
    let nonzero = wram.iter().filter(|b| **b != 0).count();

    // --find scans every aligned longword in the aperture. Unaligned candidates
    // are included too: a frame built by `movem` is word-aligned but a table
    // built with byte writes need not be, and finding nothing is a weaker
    // result than finding something.
    let matches = find.map(|value| {
        let mut out = Vec::new();
        for off in 0..wram.len().saturating_sub(3) {
            let v = u32::from_be_bytes([wram[off], wram[off + 1], wram[off + 2], wram[off + 3]]);
            if v == value {
                out.push(MatchView {
                    address: format!("0x{:08X}", 0x00FF_0000u32.wrapping_add(off as u32)),
                    wram_offset: format!("0x{off:04X}"),
                    value: format!("0x{v:08X}"),
                });
            }
        }
        out
    });

    let hex = range.map(|(addr, len)| {
        let start = fold_to_wram(addr) as usize;
        let mut rows = Vec::new();
        let mut off = start;
        let end = start + len as usize;
        while off < end {
            let take = 16.min(end - off);
            let mut s = String::new();
            for i in 0..take {
                s.push_str(&format!("{:02X} ", wram[(off + i) % wram.len()]));
            }
            rows.push(HexRow {
                address: format!("0x{:08X}", 0x00FF_0000u32 + (off % wram.len()) as u32),
                wram_offset: format!("0x{:04X}", off % wram.len()),
                bytes: s.trim_end().to_string(),
            });
            off += take;
        }
        rows
    });

    let mut warnings = Vec::new();
    if matches.as_ref().is_some_and(|m| m.is_empty()) {
        warnings.push(Warning::new(
            "no_matches",
            "no longword in work RAM holds that value; either it was never \
             written, it was written somewhere else, or the run did not reach \
             the code that writes it",
        ));
    }

    let data = RamData {
        state_artifact: super::state::save(&system, save_state, context)?,
        starting_frame,
        elapsed_frames: system.vdp.frame - starting_frame,
        termination: termination.into(),
        exceptions,
        instructions: executed,
        stopped_at: stopped_at.map(|a| format!("0x{a:08X}")),
        pc: format!("0x{:08X}", system.cpu.pc),
        registers: crate::commands::boot::Registers::of(&system),
        wram_bytes_nonzero: nonzero,
        wram_bytes_total: wram.len(),
        matches,
        hex,
    };

    let mut coverage = Coverage::complete(vec![format!(
        "instructions[0..{executed}] wram[0..{}]",
        data.wram_bytes_total
    )]);
    coverage.limitations.push(
        "instruction budget counts attempted CPU steps, including exceptions and interrupt entry"
            .into(),
    );
    if !system.cpu.branch_override.is_empty() {
        coverage.limitations.push("analyst-forced branch outcomes preserved from machine state; modified-execution diagnostic".into());
    }
    if let Some((addr, len)) = range {
        coverage.conditions.push(format!("WRAM range at folded offset 0x{:04X}, {len} bytes; wraps at 64 KiB; row addresses use canonical 0xFF0000 aliases", fold_to_wram(addr)));
    }
    if termination == "cancelled"
        || termination == "timeout"
        || (stop_at.is_some() && stopped_at.is_none())
    {
        coverage.status = crate::envelope::CoverageStatus::Partial;
        coverage.limitations.push(format!(
            "run terminated with {termination}; requested stop point may not have been reached"
        ));
    }
    let envelope = Envelope::ok(TOOL, Some(subject), coverage, data);
    let code = envelope.exit.code();
    Ok((envelope, code))
}
