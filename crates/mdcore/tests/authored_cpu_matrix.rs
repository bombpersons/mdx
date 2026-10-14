//! Frozen observations of original authored CPU programs in unchanged GPGX.
//!
//! MOVEP was once missing from dispatch and ran as an immediate operation,
//! consuming the wrong number of extension words (Lost Vikings, Boogerman).
//! TAS sat behind an unreachable match arm, so every form raised illegal
//! instruction (Ex-Mutants, Caesars Palace).
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

/// Run an authored case to its completion marker without any exception.
fn run(case: &Value, code: &[u16]) -> System {
    let bytes = authored_rom(code);
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        case["rom_sha256"].as_str().unwrap(),
        "authored ROM identity: {case}"
    );
    let mut system = System::new(Cartridge::from_bytes("authored-cpu.md", bytes).unwrap());
    for _ in 0..1000 {
        assert_eq!(system.step_instruction().1, None, "exception: {case}");
        if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
            break;
        }
    }
    assert_eq!(system.wram[0xf040..0xf042], [0xc0, 0xde], "incomplete: {case}");
    system
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn word(system: &System, at: usize) -> u64 {
    u16::from_be_bytes([system.wram[at], system.wram[at + 1]]).into()
}

#[test]
fn movep_transfers_and_counter_phase_match_independent_observations() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/movep_matrix.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let disp = case["displacement"].as_u64().unwrap() as u16;
        let mut code = vec![0x41f9, 0x00ff, 0xe000, 0x223c, 0x1122, 0x3344];
        code.extend([0x03c8, disp, 0x0388, disp + 9]);
        code.extend([0x243c, 0xaaaa, 0xaaaa, 0x263c, 0x5555, 0x5555]);
        code.extend([0x0508, disp, 0x0748, disp + 9]);
        code.extend([0x23c2, 0x00ff, 0xf000, 0x23c3, 0x00ff, 0xf004]);
        for _ in 0..case["repeat"].as_u64().unwrap() {
            code.extend([0x03c8, disp]);
        }
        code.extend([0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf008]);
        code.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
        let system = run(case, &code);
        let expected = &case["expected"];
        assert_eq!(hex(&system.wram[0xe000..0xe020]), expected["transfer"], "{case}");
        assert_eq!(hex(&system.wram[0xf000..0xf008]), expected["registers"], "{case}");
        assert_eq!(word(&system, 0xf008), expected["hv"].as_u64().unwrap(), "HV: {case}");
    }
}

#[test]
fn tas_flags_writeback_and_counter_phase_match_independent_observations() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/tas_matrix.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let (base, tas): (u32, &[u16]) = match case["mode"].as_str().unwrap() {
            "d" => (0xffe000, &[0x4ac1]),
            "ai" => (0xffe000, &[0x4ad0]),
            "pi" => (0xffe000, &[0x4ad8]),
            "pd" => (0xffe001, &[0x4ae0]),
            "di" => (0xffdff0, &[0x4ae8, 0x0010]),
            "ix" => (0xffdff0, &[0x4af0, 0x2000]),
            "aw" => (0xffe000, &[0x4af8, 0xe000]),
            "al" => (0xffe000, &[0x4af9, 0x00ff, 0xe000]),
            mode => panic!("unknown mode {mode}"),
        };
        let value = case["value"].as_u64().unwrap() as u16;
        let mut code = vec![0x13fc, value, 0x00ff, 0xe000, 0x223c, 0x5555, 0x5500 | value];
        code.extend([0x41f9, (base >> 16) as u16, base as u16, 0x7410, 0x44fc, 0x000f]);
        code.extend(tas);
        code.extend([0x40f9, 0x00ff, 0xf000, 0x23c1, 0x00ff, 0xf002, 0x23c8, 0x00ff, 0xf006]);
        for _ in 0..case["repeat"].as_u64().unwrap() {
            code.extend(tas);
        }
        code.extend([0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf00a]);
        code.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
        let system = run(case, &code);
        let expected = &case["expected"];
        assert_eq!(u64::from(system.wram[0xe000]), expected["operand"], "{case}");
        assert_eq!(word(&system, 0xf000), expected["sr"].as_u64().unwrap(), "SR: {case}");
        assert_eq!(hex(&system.wram[0xf002..0xf006]), expected["d1"], "{case}");
        assert_eq!(hex(&system.wram[0xf006..0xf00a]), expected["a0"], "{case}");
        assert_eq!(word(&system, 0xf00a), expected["hv"].as_u64().unwrap(), "HV: {case}");
    }
}

/// ADDX/SUBX/NEGX (all sizes, register and memory), CHK, divide by zero,
/// TRAPV and LINK/UNLK A7. Vectors 5-7 enter a handler that records each
/// stacked SR and return PC; cases carry their exact program words.
#[test]
fn extended_arithmetic_and_trap_class_exceptions_match_independent_observations() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/arith_trap_matrix.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let code: Vec<u16> = case["code"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_u64().unwrap() as u16)
            .collect();
        let mut bytes = authored_rom(&code);
        for (i, word) in [0x3cd7u16, 0x2cef, 0x0002, 0x4e73].iter().enumerate() {
            bytes[0x1000 + i * 2..0x1002 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
        for vector in 5..=7 {
            bytes[vector * 4..vector * 4 + 4].copy_from_slice(&0x1000u32.to_be_bytes());
        }
        let checksum = bytes[0x200..].chunks_exact(2).fold(0u16, |sum, pair| {
            sum.wrapping_add(u16::from_be_bytes([pair[0], pair[1]]))
        });
        bytes[0x18e..0x190].copy_from_slice(&checksum.to_be_bytes());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            case["rom_sha256"].as_str().unwrap(),
            "authored ROM identity: {}",
            case["family"]
        );
        let mut system = System::new(Cartridge::from_bytes("authored-arith.md", bytes).unwrap());
        for _ in 0..20000 {
            system.step_instruction();
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        let family = &case["family"];
        assert_eq!(system.wram[0xf040..0xf042], [0xc0, 0xde], "incomplete: {family}");
        let expected = &case["expected"];
        assert_eq!(hex(&system.wram[0x9000..0xa000]), expected["results"], "results: {family}");
        assert_eq!(hex(&system.wram[0xe100..0xe200]), expected["frames"], "frames: {family}");
        assert_eq!(word(&system, 0xf00a), expected["hv"].as_u64().unwrap(), "HV: {family}");
    }
}
