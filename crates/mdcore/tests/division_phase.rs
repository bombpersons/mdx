//! Operand, addressing-mode, result and SR samples captured by unchanged GPGX.
use mdcore::{Cartridge, System};

#[test]
fn division_results_flags_and_counter_phases_match_independent_programs() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/division_hv_samples.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let dividend = case["dividend"].as_u64().unwrap() as u32;
        let divisor = case["divisor"].as_u64().unwrap() as u16;
        let mut code: Vec<u16> = vec![0x33fc, 0x8104, 0x00c0, 0x0004];
        if case["wide"].as_bool().unwrap() {
            code.extend([0x33fc, 0x8c81, 0x00c0, 0x0004]);
        }
        code.extend([
            0x263c, 0, divisor, 0x2c7c, 0x00ff, 0x8000, 0x33fc, divisor, 0x00ff, 0x8000,
        ]);
        let ea = case["ea"].as_str().unwrap();
        for sample in 0..32u16 {
            code.extend([0x243c, (dividend >> 16) as u16, dividend as u16]);
            let opcode = if case["signed"].as_bool().unwrap() {
                0x85c0
            } else {
                0x84c0
            } | match ea {
                "reg" => 3,
                "imm" => 0x3c,
                "mem" => 0x16,
                _ => unreachable!(),
            };
            code.push(opcode);
            if ea == "imm" {
                code.push(divisor);
            }
            code.extend([
                0x40c4,
                0x33c4,
                0x00ff,
                0xf200 + sample * 2,
                0x23c2,
                0x00ff,
                0xf100 + sample * 4,
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
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        for (i, word) in code.iter().enumerate() {
            rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
        let mut s = System::new(Cartridge::from_bytes("division-phase.md", rom).unwrap());
        for _ in 0..10000 {
            assert_eq!(s.step_instruction().1, None);
            if s.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(&s.wram[0xf040..0xf042], &[0xc0, 0xde]);
        for (key, base, width) in [("hv", 0xf000, 2), ("results", 0xf100, 4), ("sr", 0xf200, 2)] {
            for (sample, expected) in case["expected"][key].as_array().unwrap().iter().enumerate() {
                let at = base + sample * width;
                let actual = s.wram[at..at + width]
                    .iter()
                    .fold(0u64, |value, byte| value * 256 + u64::from(*byte));
                assert_eq!(
                    actual,
                    expected.as_u64().unwrap(),
                    "{key} sample={sample}: {case}"
                );
            }
        }
    }
}
