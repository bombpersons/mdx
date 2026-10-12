//! `mdx disasm` — ask the emulator what it thinks the bytes mean.
//!
//! This exists because a trace reports the bytes the emulator *executed*
//! while this reports what the emulator *believes* they mean, and a
//! disagreement between the two names a decoder bug immediately. Reading a
//! hex dump and decoding it by hand does neither: it produces a second
//! opinion, and a second opinion that disagrees with the trace sends you
//! looking for a fetch bug that is not there.
//!
//! The decode mirrors `exec.rs`. It deliberately does not consult the manual:
//! where the two would differ, the emulator is the thing that ran the
//! cartridge, so the emulator is right and the manual needs revisiting.

use std::path::Path;

use mdcore::cartridge::Cartridge;
use mdcore::cpu::disasm;
use serde::Serialize;

use crate::envelope::{Coverage, Envelope, Failure, Subject, Warning};

pub const TOOL: &str = "mdx/disasm";

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Insn {
    pub address: String,
    /// Raw bytes, space separated, exactly as they sit in the image.
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

    // 68000 addresses are 24 bits; the image is addressed directly and wraps
    // within the cartridge the way the bus does.
    let base = (from & 0x00FF_FFFF) as usize;
    let mut instructions = Vec::with_capacity(count);
    let mut undecoded = 0usize;
    let mut cursor = base;
    let mut covered = 0usize;
    for _ in 0..count {
        let window: Vec<u8> = (0..10)
            .map(|i| {
                cart.data
                    .get((cursor + i) % cart.data.len())
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        let addr = 0x0000_0000u32.wrapping_add(cursor as u32);
        let ins = disasm::disassemble(&window, addr);
        let take = (ins.length as usize).clamp(2, window.len());
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
            bytes: raw,
            text: ins.text,
            length: ins.length,
            decoded: ins.decoded,
        });
        cursor = (cursor + take) % cart.data.len();
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
        count,
        instructions,
        undecoded,
        bytes_covered: covered,
    };
    let mut coverage = Coverage::complete(vec![format!("{count} instructions starting at bus address {base:#08X}, {covered} bytes consumed (cartridge wraps)")]);
    coverage.limitations.push("mdcore decoder interpretation; static decoding is not observed execution or independent opcode validation".into());
    let mut envelope = Envelope::ok(TOOL, Some(subject), coverage, data);
    envelope.warnings = warnings;
    let code = envelope.exit.code();
    Ok((envelope, code))
}
