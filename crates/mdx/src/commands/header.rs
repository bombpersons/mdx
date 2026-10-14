//! `mdx header` -- decode the cartridge header and exception vector table.

use crate::envelope::{Coverage, Envelope, Subject, Warning};
use mdcore::cartridge::Cartridge;
use mdcore::memmap::{fold_to_cart_mirrored, region, Region};
use serde::Serialize;
use std::path::Path;

pub const TOOL: &str = "mdx.header";

// The first 64 vector slots cover reset, exceptions, autovectors and traps.
// Slots 64..255 are user-defined vectors, outside this cartridge-header view.
fn vector_name(index: usize) -> &'static str {
    const EXCEPTIONS: [&str; 16] = [
        "reset_sp",
        "reset_pc",
        "bus_error",
        "address_error",
        "illegal_instruction",
        "divide_by_zero",
        "chk",
        "trapv",
        "privilege_violation",
        "trace",
        "line_a",
        "line_f",
        "reserved_12",
        "reserved_13",
        "reserved_14",
        "uninitialized_interrupt",
    ];
    const AUTOVECTORS: [&str; 8] = [
        "spurious_interrupt",
        "level_1_autovector",
        "level_2_autovector",
        "level_3_autovector",
        "level_4_autovector",
        "level_5_autovector",
        "level_6_autovector",
        "level_7_autovector",
    ];
    const TRAPS: [&str; 16] = [
        "trap_0", "trap_1", "trap_2", "trap_3", "trap_4", "trap_5", "trap_6", "trap_7", "trap_8",
        "trap_9", "trap_10", "trap_11", "trap_12", "trap_13", "trap_14", "trap_15",
    ];
    match index {
        0..=15 => EXCEPTIONS[index],
        24..=31 => AUTOVECTORS[index - 24],
        32..=47 => TRAPS[index - 32],
        _ => "reserved",
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct HeaderData {
    pub cartridge: CartridgeInfo,
    pub header: mdcore::SegaHeader,
    pub vectors: Vec<VectorView>,
    pub consistency: Consistency,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct CartridgeInfo {
    pub bytes: usize,
    pub is_power_of_two: bool,
    pub megabytes: f64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct VectorView {
    pub index: usize,
    pub name: &'static str,
    pub address: String,
    pub region: &'static str,
    /// Cartridge offset this address folds to, when it lands in ROM.
    pub cart_offset: Option<String>,
    /// Human note about anything unusual about this vector.
    pub note: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Consistency {
    pub declared_rom_len: Option<usize>,
    pub actual_rom_len: usize,
    /// `Some(true)` / `Some(false)` when the header declared a range we can
    /// check, `None` when it declared nothing at all.
    pub declared_range_matches_file: Option<bool>,
    pub sram_declared: bool,
}

pub fn run(rom: &Path) -> Result<(Envelope<HeaderData>, i32), crate::envelope::Failure> {
    let cart = Cartridge::load(rom).map_err(|e| crate::envelope::Failure {
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

    let mut warnings: Vec<Warning> = Vec::new();
    if !cart.header.unset_fields.is_empty() {
        warnings.push(Warning::new(
            "header_fields_unset",
            format!(
                "header fields left blank by the developer: {}",
                cart.header.unset_fields.join(", ")
            ),
        ));
    }
    if cart.header.domestic_name.is_empty() {
        warnings.push(Warning::new(
            "header_no_name",
            "cartridge carries no domestic name",
        ));
    }

    let rom_len = cart.data.len();
    let view: Vec<VectorView> = cart.data[..0x100]
        .chunks_exact(4)
        .enumerate()
        .map(|(index, bytes)| {
            let address = u32::from_be_bytes(bytes.try_into().unwrap());
            let r = region(address);
            let cart_offset = match r {
                Region::Cartridge => {
                    Some(format!("0x{:06X}", fold_to_cart_mirrored(address, rom_len)))
                }
                _ => None,
            };
            let note = match (index, r) {
                // A stack pointer in work RAM is correct, not suspicious. The
                // 68000 leaves reset in supervisor mode with interrupts masked,
                // so the initial stack only has to be writable RAM.
                (0, Region::Wram) => {
                    Some("reset SP is in work RAM, which is where a stack belongs".into())
                }
                (0, Region::Cartridge) => Some(
                    "reset SP decodes into the cartridge, which is not writable; \
                     this ROM cannot use the vector's value as a stack pointer"
                        .into(),
                ),
                (1, Region::Wram) => Some(
                    "reset PC decodes into work RAM, not the cartridge; this ROM's \
                     entry point is not what the vector appears to say"
                        .into(),
                ),
                // Only worth saying when it is actually odd. Reporting this for
                // every cartridge would put a note on the overwhelmingly
                // normal 0x00000200 entry point.
                (1, Region::Cartridge) if address & 1 != 0 => {
                    Some("reset PC is odd; the 68000 clears bit 0 before fetching".into())
                }
                _ => None,
            };
            VectorView {
                index,
                name: vector_name(index),
                address: format!("0x{address:08X}"),
                region: region_name(r),
                cart_offset,
                note,
            }
        })
        .collect();

    // Only a reset PC that lands outside the cartridge is genuinely
    // unresolved. Warning about the ordinary case trains an agent to ignore
    // the field, which is worse than leaving it empty.
    if view.get(1).map(|v| v.region) == Some(region_name(Region::Wram)) {
        warnings.push(
            Warning::new(
                "reset_vector_unusual",
                "the reset PC does not decode into the cartridge window; \
                 treat the boot path as unresolved until a trace says otherwise",
            )
            .at("rom+0x04"),
        );
    }

    let declared = cart.header.declared_rom_len();
    let consistency = Consistency {
        declared_rom_len: declared,
        actual_rom_len: rom_len,
        declared_range_matches_file: declared.map(|d| d == rom_len),
        sram_declared: cart.header.sram_start.is_some(),
    };

    let data = HeaderData {
        cartridge: CartridgeInfo {
            bytes: rom_len,
            is_power_of_two: cart.is_power_of_two(),
            megabytes: round2(rom_len as f64 / (1024.0 * 1024.0)),
        },
        header: cart.header.clone(),
        vectors: view,
        consistency,
    };

    let coverage = Coverage::complete(vec![
        format!("rom_bytes[0..{rom_len}]"),
        "exception_vectors[0..64] (system/trap slots; user-defined slots 64..255 excluded)".into(),
    ]);

    let mut envelope = Envelope::ok(TOOL, Some(subject), coverage, data);
    envelope.warnings = warnings;
    let code = envelope.exit.code();
    Ok((envelope, code))
}

fn region_name(r: Region) -> &'static str {
    match r {
        Region::Cartridge => "cartridge",
        Region::Z80Ram => "z80_ram",
        Region::Fm => "fm",
        Region::Io => "io",
        Region::Z80Ctrl => "z80_ctrl",
        Region::Sram => "sram",
        Region::Vdp => "vdp",
        Region::Wram => "wram",
        Region::Unmapped => "unmapped",
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
