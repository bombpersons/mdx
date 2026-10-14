//! Frozen observations of cartridge backup RAM in unchanged GPGX: default SRAM
//! for small ROMs, header-declared SRAM, the $A130F1 switch for SRAM that
//! overlaps a 4 MiB ROM, write protection and switching back to ROM.
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

fn program(length: usize, header: bool) -> Vec<u8> {
    let read = |slot: u16| [0x3039, 0x0020, 0x0000, 0x33C0, 0x00FF, 0xF000 + slot];
    let byte = |value: u16, high: u16, low: u16| [0x13FC, value, high, low];
    let mut code = read(0).to_vec();
    code.extend(byte(0x42, 0x0020, 0x0001));
    code.extend(byte(0x43, 0x0020, 0x0000));
    code.extend(read(2));
    code.extend(byte(0x01, 0x00A1, 0x30F1));
    code.extend(byte(0x44, 0x0020, 0x0001));
    code.extend(read(4));
    code.extend(byte(0x03, 0x00A1, 0x30F1));
    code.extend(byte(0x55, 0x0020, 0x0001));
    code.extend(read(6));
    code.extend(byte(0x00, 0x00A1, 0x30F1));
    code.extend(read(8));
    code.extend([0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]);
    let mut rom = authored_rom(&code);
    rom.resize(length, 0x6C);
    if header {
        rom[0x1B0..0x1B4].copy_from_slice(b"RA\xf8\x20");
        rom[0x1B4..0x1B8].copy_from_slice(&0x200001u32.to_be_bytes());
        rom[0x1B8..0x1BC].copy_from_slice(&0x203FFFu32.to_be_bytes());
    }
    rom
}

#[test]
fn backup_ram_mapping_switch_and_protection_match_independent_observations() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/sram_matrix.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let bytes = program(case["length"].as_u64().unwrap() as usize, case["header"].as_bool().unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            case["rom_sha256"].as_str().unwrap(),
            "authored ROM identity: {}",
            case["length"]
        );
        let mut system = System::new(Cartridge::from_bytes("sram.md", bytes).unwrap());
        for _ in 0..1000 {
            system.step_instruction();
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        let words: Vec<u64> = system.wram[0xf000..0xf00a]
            .chunks_exact(2)
            .map(|pair| u64::from(u16::from_be_bytes([pair[0], pair[1]])))
            .collect();
        let expected: Vec<u64> = case["expected"]["words"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap()).collect();
        assert_eq!(words, expected, "length {} header {}", case["length"], case["header"]);
    }
}
