//! Frozen observations of original authored TRAP/RTE programs in unchanged GPGX.
use mdcore::{cpu::m68000::Exception, Cartridge, System};
use sha2::{Digest, Sha256};

fn program(wide: bool, user: bool, trap: u16) -> Vec<u8> {
    let mut code = vec![0x33fc, 0x8104, 0x00c0, 0x0004];
    if wide {
        code.extend([0x33fc, 0x8c81, 0x00c0, 0x0004]);
    }
    if user {
        code.extend([0x207c, 0x00ff, 0x6000, 0x4e60, 0x46fc, 0x0001]);
    }
    for sample in 0..32u16 {
        code.extend([
            0x44fc,
            1,
            0x4e40 | trap,
            0x3039,
            0x00c0,
            0x0008,
            0x33c0,
            0x00ff,
            0xf000 + sample * 2,
        ]);
        code.extend([0x4e71; 13]);
    }
    code.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
    let handler = [
        0x3417, 0x262f, 2, 0x280f, 0x40c5, 0x33c2, 0x00ff, 0xf100, 0x23c3, 0x00ff, 0xf104, 0x23c4,
        0x00ff, 0xf108, 0x33c5, 0x00ff, 0xf10c, 0x4e73,
    ];
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
    rom[0x78..0x7c].copy_from_slice(&0x1000u32.to_be_bytes());
    let vector = (32 + trap as usize) * 4;
    rom[vector..vector + 4].copy_from_slice(&0x1000u32.to_be_bytes());
    for (at, words) in [(0x200, code.as_slice()), (0x1000, handler.as_slice())] {
        for (i, word) in words.iter().enumerate() {
            rom[at + i * 2..at + i * 2 + 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let checksum = rom[0x200..].chunks_exact(2).fold(0u16, |sum, pair| {
        sum.wrapping_add(u16::from_be_bytes([pair[0], pair[1]]))
    });
    rom[0x18e..0x190].copy_from_slice(&checksum.to_be_bytes());
    rom
}

#[test]
fn all_software_traps_match_independent_stack_return_and_counter_observations() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/software_trap_hv.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let trap = case["trap"].as_u64().unwrap() as u16;
        let bytes = program(
            case["wide"].as_bool().unwrap(),
            case["user"].as_bool().unwrap(),
            trap,
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            case["rom_sha256"].as_str().unwrap(),
            "authored ROM identity: {case}"
        );
        let mut system = System::new(Cartridge::from_bytes("authored-trap.md", bytes).unwrap());
        let mut observed = 0;
        for _ in 0..10000 {
            if let Some(exception) = system.step_instruction().1 {
                assert_eq!(
                    exception,
                    Exception::Vector {
                        vector: 32 + trap as u8
                    }
                );
                observed += 1;
            }
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(
            system.wram[0xf040..0xf042],
            [0xc0, 0xde],
            "incomplete: {case}"
        );
        assert_eq!(observed, 32);
        let read = |at, width| {
            system.wram[at..at + width]
                .iter()
                .fold(0u64, |n, b| n * 256 + u64::from(*b))
        };
        for (key, at, width) in [
            ("stacked_sr", 0xf100, 2),
            ("return_pc", 0xf104, 4),
            ("handler_sp", 0xf108, 4),
            ("handler_sr", 0xf10c, 2),
        ] {
            assert_eq!(
                read(at, width),
                case["expected"][key].as_u64().unwrap(),
                "{key}: {case}"
            );
        }
        for (i, expected) in case["expected"]["hv"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(
                read(0xf000 + i * 2, 2),
                expected.as_u64().unwrap(),
                "HV sample {i}: {case}"
            );
        }
    }
}

#[test]
fn a_user_trap_checkpoint_preserves_the_supervisor_frame_and_resumes_exactly() {
    let cart = Cartridge::from_bytes("authored-user-trap.md", program(true, true, 15)).unwrap();
    let mut original = System::new(cart.clone());
    for _ in 0..100 {
        if original.step_instruction().1.is_some() {
            break;
        }
    }
    assert_eq!(original.cpu.pc, 0x1000);
    assert_eq!(original.cpu.a[7], 0xff7ffa);
    assert_eq!(original.cpu.usp, 0xff6000);
    let state = original.save_state().unwrap();
    let mut resumed = System::from_state(cart, state).unwrap();
    for _ in 0..1000 {
        assert_eq!(original.step_instruction(), resumed.step_instruction());
    }
    assert_eq!(
        serde_json::to_vec(&original.save_state().unwrap()).unwrap(),
        serde_json::to_vec(&resumed.save_state().unwrap()).unwrap()
    );
}
