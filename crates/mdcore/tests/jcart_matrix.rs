//! Frozen observations of unchanged GPGX reading the J-Cart extra controller
//! ports at $380000 (word, odd and even bytes) with TH high and low.
use mdcore::input::Button;
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

fn program() -> Vec<u8> {
    let mut code = Vec::new();
    for (i, th) in [1u16, 0, 1].into_iter().enumerate() {
        let at = 0xF000 + i as u16 * 4;
        code.extend([0x33FC, th, 0x0038, 0x0000]);
        code.extend([0x3039, 0x0038, 0x0000, 0x33C0, 0x00FF, at]);
        code.extend([0x1039, 0x0038, 0x0001, 0x13C0, 0x00FF, at + 2]);
        code.extend([0x1039, 0x0038, 0x0000, 0x13C0, 0x00FF, at + 3]);
    }
    code.extend([0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]);
    let mut rom = authored_rom(&code);
    rom[0x180..0x18E].copy_from_slice(b"GM T-120066-00");
    rom
}

fn mask(buttons: &Value) -> u8 {
    buttons.as_array().unwrap().iter().fold(0, |m, b| {
        let button: Button = serde_json::from_value(b.clone()).unwrap();
        m | button.mask()
    })
}

#[test]
fn jcart_ports_match_independent_observations() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/jcart_matrix.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let bytes = program();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), case["rom_sha256"].as_str().unwrap());
        let mut system = System::new(Cartridge::from_bytes("jcart.md", bytes).unwrap());
        assert!(system.cart_hw.has_jcart());
        for event in case["replay"].as_array().unwrap() {
            system.set_pad(event["port"].as_u64().unwrap() as usize, mask(&event["buttons"]));
        }
        for _ in 0..1000 {
            system.step_instruction();
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        let reads: String = system.wram[0xf000..0xf00c].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(reads, case["expected"]["reads"].as_str().unwrap(), "{}", case["replay"]);
    }
}
