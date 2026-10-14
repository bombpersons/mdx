//! Frozen PAL observations of unchanged GPGX: a European-header ROM samples the
//! HV counter and VDP status 32 times across a frame in V28 and V30 modes.
//! HV values must match exactly, as must every status bit except HBLANK: the
//! reference evaluates status at the end of the reading instruction using its
//! opcode cycle table, which mdcore does not model, so HBLANK can differ for a
//! read within one instruction of a blanking edge.
use mdcore::{Cartridge, System};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The probe harness overlays each program on its default HV sampler, so the
/// sampler's tail remains in the image beyond the authored code.
fn authored_rom(code: &[u16]) -> Vec<u8> {
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
    for words in [sampler.as_slice(), code] {
        for (i, word) in words.iter().enumerate() {
            rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let checksum = rom[0x200..].chunks_exact(2).fold(0u16, |sum, pair| {
        sum.wrapping_add(u16::from_be_bytes([pair[0], pair[1]]))
    });
    rom[0x18e..0x190].copy_from_slice(&checksum.to_be_bytes());
    rom
}

fn program(mode: u16, delay: u16) -> Vec<u8> {
    let mut code = vec![0x33FC, mode, 0x00C0, 0x0004];
    for i in 0..32u16 {
        code.extend([0x3239, 0x00C0, 0x0004, 0x33C1, 0x00FF, 0xF100 + i * 2]);
        code.extend([0x3039, 0x00C0, 0x0008, 0x33C0, 0x00FF, 0xF000 + i * 2]);
        code.extend([0x323C, delay, 0x51C9, 0xFFFE]);
    }
    code.extend([0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]);
    let mut rom = authored_rom(&code);
    rom[0x1F0..0x1F3].copy_from_slice(b"E  ");
    rom
}

#[test]
fn pal_hv_counter_and_status_match_independent_observations() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/pal_phase.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mode = case["mode"].as_u64().unwrap() as u16;
        let delay = case["delay"].as_u64().unwrap() as u16;
        let bytes = program(mode, delay);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), case["rom_sha256"].as_str().unwrap());
        let mut system = System::new(Cartridge::from_bytes("pal.md", bytes).unwrap());
        assert!(system.vdp.pal, "European header selects a PAL console");
        for _ in 0..200_000 {
            system.step_instruction();
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(system.wram[0xf040..0xf042], [0xc0, 0xde]);
        let words = |at: usize| -> Vec<u64> {
            system.wram[at..at + 64].chunks_exact(2).map(|p| u64::from(u16::from_be_bytes([p[0], p[1]]))).collect()
        };
        let list = |key: &str| -> Vec<u64> {
            case["expected"][key].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect()
        };
        assert_eq!(words(0xf000), list("hv"), "HV mode {mode:#X} delay {delay}");
        let masked = |v: Vec<u64>| -> Vec<u64> { v.into_iter().map(|s| s & !4).collect() };
        assert_eq!(masked(words(0xf100)), masked(list("status")), "status mode {mode:#X} delay {delay}");
    }
}
