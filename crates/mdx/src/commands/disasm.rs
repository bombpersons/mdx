//! `mdx disasm` — ask the emulator what it thinks the bytes mean.
//!
//! This exists because a trace reports the bytes the emulator *executed*
//! while this reports what the emulator *believes* they mean, and a
//! disagreement between the two names a decoder bug immediately. Reading a
//! hex dump and decoding it by hand does neither: it produces a second
//! opinion, and a second opinion that disagrees with the trace sends you
//! looking for a fetch bug that is not there.
//!
//! The decode mirrors `exec.rs`; agreement between these two interpretations
//! is not independent validation of the CPU implementation.

use std::path::Path;

use mdcore::cartridge::Cartridge;
use mdcore::cpu::disasm;
use mdcore::memmap::{fold_to_cart_mirrored, region, Region};
use serde::Serialize;

use crate::envelope::{Coverage, Envelope, Failure, Subject, Warning};

pub const TOOL: &str = "mdx/disasm";

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Insn {
    pub address: String,
    /// ROM offset selected by this bus address under mdcore's cartridge mapping.
    pub cart_offset: String,
    /// Emulated bus bytes, space separated; unbacked ROM-tail bytes are FF.
    pub bytes: String,
    /// What mdcore's decoder makes of them.
    pub text: String,
    /// Bytes consumed, which is what makes a listing advance.
    pub length: u8,
    /// False when the decoder declined, in which case `text` is `??` and
    /// `length` is the only thing it is willing to commit to.
    pub decoded: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct DisasmData {
    pub from: String,
    pub cart_offset: String,
    pub count: usize,
    pub instructions: Vec<Insn>,
    pub undecoded: usize,
    /// Bytes consumed in total, so a caller can check the listing advanced the
    /// way it expected to.
    pub bytes_covered: usize,
}

pub fn run(rom: &Path, from: u32, count: usize) -> Result<(Envelope<DisasmData>, i32), Failure> {
    if from & 1 != 0 || from > 0xFF_FFFF || count > 4096 {
        return Err(Failure {
            code: "usage".into(),
            message: "disassembly requires an even 24-bit address and count <= 4096".into(),
            remediation: None,
        });
    }
    if region(from) != Region::Cartridge {
        return Err(Failure {
            code: "invalid_arguments".into(),
            message: "disasm.from must be in the cartridge CPU bus window 0x000000..0x3FFFFF".into(),
            remediation: Some("use a cartridge bus address, normally equal to its ROM offset; RAM requires a captured-memory view".into()),
        });
    }
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

    // Keep the original bus PC even when the cartridge offset is mirrored.
    let base = from;
    let mut instructions = Vec::with_capacity(count);
    let mut undecoded = 0usize;
    let mut cursor = base;
    let mut covered = 0usize;
    let mut missing_bytes = 0usize;
    for _ in 0..count {
        if region(cursor) != Region::Cartridge {
            break;
        }
        let window: Vec<u8> = (0..10)
            .map(|i| {
                let address = cursor.wrapping_add(i) & 0xFF_FFFF;
                if region(address) != Region::Cartridge {
                    return 0xFF;
                }
                cart.mapped_byte(address)
            })
            .collect();
        let addr = cursor;
        let ins = disasm::disassemble(&window, addr);
        let take = (ins.length as usize).clamp(2, window.len());
        // Only consumed bytes affect coverage; lookahead may cross a boundary.
        missing_bytes += (0..take)
            .filter(|i| {
                let address = addr.wrapping_add(*i as u32) & 0xFF_FFFF;
                region(address) != Region::Cartridge || image_offset(address, cart.data.len()) >= cart.data.len()
            })
            .count();
        let raw = window[..take]
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        if !ins.decoded {
            undecoded += 1;
        }
        instructions.push(Insn {
            address: format!("0x{addr:06X}"),
            cart_offset: format!("0x{:06X}", image_offset(addr, cart.data.len())),
            bytes: raw,
            text: ins.text,
            length: ins.length,
            decoded: ins.decoded,
        });
        cursor = cursor.wrapping_add(take as u32) & 0xFF_FFFF;
        covered += take;
    }

    let mut warnings = Vec::new();
    if undecoded > 0 {
        warnings.push(Warning::new(
            "undecoded_instructions",
            &format!(
                "{undecoded} of {count} instructions are not decoded; each is \
                 reported as `??` and advances the listing by the two bytes an \
                 instruction always occupies. That is a coverage gap in the \
                 decoder, not in the cartridge - a listing assembled from data \
                 rather than code will look the same, so check the address \
                 before treating it as a bug."
            ),
        ));
    }

    let data = DisasmData {
        from: format!("0x{base:06X}"),
        cart_offset: format!("0x{:06X}", image_offset(base, cart.data.len())),
        count,
        instructions,
        undecoded,
        bytes_covered: covered,
    };
    let mut coverage = Coverage::complete(vec![
        format!("{} of {count} requested instructions starting at bus address {base:#08X}, {covered} bytes consumed", data.instructions.len()),
        "cartridge CPU bus window 0x000000..0x3FFFFF; mdcore mapping folds at a power-of-two period (minimum 1 KiB), with unbacked tail bytes represented as FF; bus PC does not fold".into(),
    ]);
    if data.instructions.len() != count || missing_bytes != 0 {
        coverage.status = crate::envelope::CoverageStatus::Partial;
        coverage.limitations.push(format!("listing includes a cartridge-window boundary or unbacked ROM tail; {missing_bytes} consumed unavailable bytes represented as FF"));
    }
    coverage.limitations.push("mdcore decoder interpretation; static decoding is not observed execution or independent opcode validation".into());
    let mut envelope = Envelope::ok(TOOL, Some(subject), coverage, data);
    envelope.warnings = warnings;
    let code = envelope.exit.code();
    Ok((envelope, code))
}

fn image_offset(address: u32, rom_len: usize) -> usize {
    // Preserve the mapped offset even if it lies in the undumped FF tail.
    fold_to_cart_mirrored(address, rom_len) as usize
}

/// Where Z80 bytes come from.
pub enum Z80Source<'a> {
    /// ROM file offset of `from`; `origin` is the Z80 address of that byte.
    Rom { origin: u16 },
    /// Z80 RAM of a machine state; `from` is a Z80 address.
    State(&'a str),
}

/// Disassemble Z80 code, e.g. a sound driver in the ROM before upload or the
/// Z80 RAM of a saved machine state.
pub fn run_z80(
    rom: &Path,
    from: u32,
    count: usize,
    source: Z80Source,
    context: &crate::context::Context,
) -> Result<(Envelope<DisasmData>, i32), Failure> {
    if count > 4096 {
        return Err(crate::artifacts::failure("limit_exceeded", "disasm count must be <= 4096"));
    }
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
        architecture: Some("z80".into()),
    };
    // (bytes, index of `from` within them, Z80 address of `from`, source label)
    let (memory, start, origin, label): (Vec<u8>, usize, u16, String) = match source {
        Z80Source::Rom { origin } => {
            if from as usize >= cart.data.len() {
                return Err(crate::artifacts::failure(
                    "invalid_arguments",
                    "for cpu z80 without from_state, from is a ROM file offset inside the image",
                ));
            }
            (cart.data.clone(), from as usize, origin, "ROM file".into())
        }
        Z80Source::State(state) => {
            if from > 0x1FFF {
                return Err(crate::artifacts::failure(
                    "invalid_arguments",
                    "with from_state, from is a Z80 RAM address 0x0000..0x1FFF",
                ));
            }
            let system = super::state::load(cart, Some(state), super::state::RegionChoice::Auto, context)?;
            (system.z80_ram.clone(), from as usize, from as u16, "machine-state Z80 RAM".into())
        }
    };
    let mut instructions = Vec::with_capacity(count);
    let (mut at, mut undecoded, mut missing) = (start, 0usize, 0usize);
    while instructions.len() < count && at < memory.len() {
        let window: Vec<u8> = (0..4).map(|i| memory.get(at + i).copied().unwrap_or(0)).collect();
        let address = origin.wrapping_add((at - start) as u16);
        let insn = mdcore::z80::disasm::disassemble(&window, address);
        let length = usize::from(insn.length);
        missing += (at + length).saturating_sub(memory.len());
        undecoded += usize::from(!insn.decoded);
        instructions.push(Insn {
            address: format!("0x{address:04X}"),
            cart_offset: format!("0x{at:06X}"),
            bytes: window[..length].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
            text: insn.text,
            length: insn.length,
            decoded: insn.decoded,
        });
        at += length;
    }
    let covered = at - start;
    let mut coverage = Coverage::complete(vec![
        format!("{} of {count} Z80 instructions from {label} offset {start:#06X} at Z80 address {origin:#06X}, {covered} bytes", instructions.len()),
        "cart_offset is the offset within the source (ROM file or 8 KiB Z80 RAM)".into(),
    ]);
    if instructions.len() != count || missing != 0 {
        coverage.status = crate::envelope::CoverageStatus::Partial;
        coverage.limitations.push(format!("listing reached the end of the source; {missing} missing bytes read as 00"));
    }
    coverage.limitations.push("mdcore Z80 decoder; lengths are cross-checked against the executor, mnemonics are not independently validated".into());
    let mut envelope = Envelope::ok(TOOL, Some(subject), coverage, DisasmData {
        from: format!("0x{origin:04X}"),
        cart_offset: format!("0x{start:06X}"),
        count,
        instructions,
        undecoded,
        bytes_covered: covered,
    });
    if undecoded > 0 {
        envelope.warnings.push(Warning::new(
            "undecoded_instructions",
            &format!("{undecoded} undefined ED opcodes or redundant prefixes, which the Z80 executes as no-ops; check whether this is data"),
        ));
    }
    let code = envelope.exit.code();
    Ok((envelope, code))
}
