//! Frozen observations of unchanged GPGX bit-banging a header-declared serial
//! EEPROM (Sega board, X24C01) at $200001: page-write wrap, sequential reads
//! with array rollover, and every ACK/data level the 68000 sees.
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

#[test]
fn serial_eeprom_protocol_matches_independent_observations() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/eeprom_matrix.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let code: Vec<u16> = case["code"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap() as u16).collect();
        let mut bytes = authored_rom(&code);
        bytes[0x1B0..0x1B4].copy_from_slice(b"RA\xe8\x40");
        bytes[0x1B4..0x1BC].copy_from_slice(&[0x00, 0x20, 0x00, 0x01, 0x00, 0x20, 0x00, 0x01]);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), case["rom_sha256"].as_str().unwrap());
        let mut system = System::new(Cartridge::from_bytes("eeprom.md", bytes).unwrap());
        assert!(system.cart_hw.eeprom.is_some(), "header declares a serial EEPROM");
        for _ in 0..100_000 {
            system.step_instruction();
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(system.wram[0xf040..0xf042], [0xc0, 0xde]);
        let sda: String = system.wram[0xf000..0xf040].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(sda, case["expected"]["sda"].as_str().unwrap(), "address {}", case["address"]);
    }
}
