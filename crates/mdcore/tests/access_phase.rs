//! Frozen observations of unchanged GPGX (tools/probes/access_phase_matrix.py):
//! the VDP status word sampled at dispatch plus the reading opcode's table
//! cost, the one-instruction interrupt delay after a register write enables a
//! pending VINT (none for MOVE.L), and YM2612 BUSY through 68000 Z80-bus
//! latency with frame-relative rounding (also through Z80 accesses), HINT
//! sequencing at line 0 and the VBLANK edge, and the late DMA-fill start whose
//! first-update budget wraps, and 3-/6-button pad reads (header-selected
//! type, TH pulse counter and timeout, TH input switching latency), and
//! control-port commands (register writes retarget data writes; A15-A14 latch),
//! and 68000 address errors (group-0 frame, registers at the fault, timing).
use mdcore::{Cartridge, System};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Probe harness image: default HV sampler overlaid by the authored code, plus
/// an optional level-6 handler at $1000.
fn authored_rom(code: &[u16], handler: Option<&[u16]>) -> Vec<u8> {
    let mut rom = vec![0; 65536];
    rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    rom[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    for base in [0x120, 0x150] {
        rom[base..base + 48].fill(b' ');
        let title = b"HV counter phase probe";
        rom[base..base + title.len()].copy_from_slice(title);
    }
    rom[0x1a4..0x1a8].copy_from_slice(&0xffffu32.to_be_bytes());
    rom[0x1b0..0x1b4].copy_from_slice(&0xff0000u32.to_be_bytes());
    rom[0x1b4..0x1b8].copy_from_slice(&0xffffffu32.to_be_bytes());
    rom[0x1f0..0x1f3].copy_from_slice(b"U  ");
    let mut sampler = vec![0x33fc, 0x8104, 0x00c0, 0x0004];
    for sample in 0..32u16 {
        sampler.extend([0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000 + sample * 2]);
        sampler.extend([0x4e71; 13]);
    }
    sampler.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
    if handler.is_some() {
        rom[0x78..0x7c].copy_from_slice(&0x1000u32.to_be_bytes());
    }
    for (at, words) in [(0x200, sampler.as_slice()), (0x200, code), (0x1000, handler.unwrap_or(&[]))] {
        for (i, word) in words.iter().enumerate() {
            rom[at + i * 2..at + 2 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let checksum = rom[0x200..].chunks_exact(2).fold(0u16, |sum, pair| {
        sum.wrapping_add(u16::from_be_bytes([pair[0], pair[1]]))
    });
    rom[0x18e..0x190].copy_from_slice(&checksum.to_be_bytes());
    rom
}

const HANDLER: [u16; 7] = [0x23EF, 0x0002, 0x00FF, 0xF020, 0x5278, 0xF030, 0x4E73];
const AERR_HANDLER: [u16; 17] = [
    0x32F9, 0x00C0, 0x0008, 0x32DF, 0x32DF, 0x32DF, 0x32DF, 0x32DF, 0x32DF, 0x32DF, 0x22C8, 0x22C0,
    0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE,
];
const HINT_HANDLER: [u16; 8] = [0xB3FC, 0x00FF, 0xF040, 0x6406, 0x32F9, 0x00C0, 0x0008, 0x4E73];

#[test]
fn access_phases_match_independent_observations() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/access_phase.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 172);
    for case in cases {
        let code: Vec<u16> =
            case["code"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap() as u16).collect();
        let handler = match case["kind"].as_str() {
            Some("irq") => Some(&HANDLER[..]),
            Some("hint") => Some(&HINT_HANDLER[..]),
            Some("aerr") => Some(&AERR_HANDLER[..]),
            _ => None,
        };
        let mut bytes = authored_rom(&code, handler);
        if case["kind"] == "pad" {
            let io: &[u8] = if case["six"] == true { b"J6" } else { b"J " };
            bytes[0x190..0x192].copy_from_slice(io);
        }
        if case["kind"] == "aerr" {
            // Address error vector instead of level 6.
            bytes.copy_within(0x78..0x7c, 0x0C);
            bytes[0x78..0x7c].fill(0);
        }
        if case["kind"] == "hint" {
            // Level-4 autovector instead of level 6.
            bytes.copy_within(0x78..0x7c, 0x70);
            bytes[0x78..0x7c].fill(0);
        }
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), case["rom_sha256"].as_str().unwrap());
        let mut system = System::new(Cartridge::from_bytes("access.md", bytes).unwrap());
        for event in case["replay"].as_array().into_iter().flatten() {
            let buttons: Vec<mdcore::input::Button> =
                serde_json::from_value(event["buttons"].clone()).unwrap();
            system.io.pad_buttons[0] = buttons.iter().fold(0, |mask, b| mask | b.mask());
        }
        for _ in 0..1_000_000 {
            system.step_instruction();
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(system.wram[0xf040..0xf042], [0xc0, 0xde]);
        let out: String = system.wram[0xf000..0xf040].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(out, case["expected"]["out"].as_str().unwrap(), "{case}");
    }
}
