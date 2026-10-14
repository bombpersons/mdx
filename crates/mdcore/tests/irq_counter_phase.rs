//! Independent IRQ-entry samples expose the E-clock acknowledge wait.
use mdcore::{Cartridge, System};

fn verify_interrupts(first_only: bool) {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/irq_hv_samples.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let wide = case["wide"].as_bool().unwrap();
        let nops = case["startup_nops"].as_u64().unwrap();
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        rom[0x78..0x7c].copy_from_slice(&0x1000u32.to_be_bytes());
        let mut code: Vec<u16> = vec![0x33fc, 0x8124, 0x00c0, 0x0004];
        if wide {
            code.extend([0x33fc, 0x8c81, 0x00c0, 0x0004]);
        }
        code.extend([0x207c, 0x00ff, 0xf000, 0x7e00]);
        code.extend(std::iter::repeat_n(0x4e71, nops as usize));
        code.extend([0x46fc, 0x2000, 0x60fe]);
        let handler = [
            0x3039, 0x00c0, 0x0008, 0x30c0, 0x5247, 0x0c47, 32, 0x660c, 0x33fc, 0xc0de, 0x00ff,
            0xf040, 0x3ebc, 0x2700, 0x4e73,
        ];
        for (start, words) in [(0x200, code.as_slice()), (0x1000, &handler)] {
            for (i, word) in words.iter().enumerate() {
                rom[start + i * 2..start + i * 2 + 2].copy_from_slice(&word.to_be_bytes());
            }
        }
        let mut system = System::new(Cartridge::from_bytes("irq-phase.md", rom).unwrap());
        for _ in 0..1000000 {
            assert_eq!(system.step_instruction().1, None);
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(&system.wram[0xf040..0xf042], &[0xc0, 0xde]);
        for (sample, expected) in case["expected"].as_array().unwrap().iter().enumerate() {
            if (sample == 0) != first_only {
                continue;
            }
            let at = 0xf000 + sample * 2;
            let actual = u16::from_be_bytes([system.wram[at], system.wram[at + 1]]);
            assert_eq!(
                u64::from(actual),
                expected.as_u64().unwrap(),
                "wide={wide} startup_nops={nops} IRQ sample={sample}"
            );
        }
    }
}

#[test]
fn steady_interrupt_acknowledge_matches_independent_counter_samples() {
    verify_interrupts(false);
}

#[test]
fn first_interrupt_boundary_matches_independent_counter_samples() {
    verify_interrupts(true);
}

#[test]
fn sr_writes_acknowledge_pending_vint_before_fixed_instruction_clocks() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pending_irq_hv_samples.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        rom[0x78..0x7c].copy_from_slice(&0x1000u32.to_be_bytes());
        let mut code: Vec<u16> = vec![0x33fc, 0x8124, 0x00c0, 0x0004];
        if case["wide"].as_bool().unwrap() {
            code.extend([0x33fc, 0x8c81, 0x00c0, 0x0004]);
        }
        code.extend([0x3c3c, 2000, 0x51ce, 0xfffe]);
        code.extend(std::iter::repeat_n(
            0x4e71,
            case["nops"].as_u64().unwrap() as usize,
        ));
        match case["form"].as_str().unwrap() {
            "move" => code.extend([0x46fc, 0x2000]),
            "and" => code.extend([0x027c, 0xf8ff]),
            "eor" => code.extend([0x0a7c, 0x0700]),
            "stop" => code.extend([0x4e72, 0x2000]),
            "rte" => {
                let return_pc = 0x200 + 2 * (code.len() + 6);
                code.extend([
                    0x2f3c,
                    (return_pc >> 16) as u16,
                    return_pc as u16,
                    0x3f3c,
                    0x2000,
                    0x4e73,
                ]);
            }
            _ => unreachable!(),
        }
        code.push(0x60fe);
        let handler = [
            0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000, 0x33fc, 0xc0de, 0x00ff, 0xf040, 0x3ebc,
            0x2700, 0x4e73,
        ];
        for (start, words) in [(0x200, code.as_slice()), (0x1000, &handler)] {
            for (i, word) in words.iter().enumerate() {
                rom[start + i * 2..start + i * 2 + 2].copy_from_slice(&word.to_be_bytes());
            }
        }
        let mut s = System::new(Cartridge::from_bytes("pending-irq.md", rom).unwrap());
        for _ in 0..10000 {
            assert_eq!(s.step_instruction().1, None);
            if s.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(&s.wram[0xf040..0xf042], &[0xc0, 0xde]);
        assert_eq!(s.vdp.vint_taken, 1);
        let actual = u16::from_be_bytes([s.wram[0xf000], s.wram[0xf001]]);
        assert_eq!(
            u64::from(actual),
            case["expected"].as_u64().unwrap(),
            "{case}"
        );
    }
}
