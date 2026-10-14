//! Authored MOVEM transfer phases sampled by unchanged GPGX, including word
//! controls that must not suppress refresh. No commercial ROM is required.
use mdcore::{Cartridge, System};

#[test]
fn movem_refresh_matches_independent_counter_samples() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/movem_hv_samples.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let opcode = case["opcode"].as_u64().unwrap() as u16;
        let mask = case["mask"].as_u64().unwrap() as u16;
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        let mut code = vec![0x33fc, 0x8104, 0x00c0, 0x0004];
        if case["wide"].as_bool().unwrap() {
            code.extend([0x33fc, 0x8c81, 0x00c0, 0x0004]);
        }
        code.extend([0x2c7c, 0x00ff, 0x8000]);
        for sample in 0..32 {
            code.extend([
                opcode,
                mask,
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
        for (i, word) in code.iter().enumerate() {
            rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
        let mut s = System::new(Cartridge::from_bytes("movem-phase.md", rom).unwrap());
        for _ in 0..10000 {
            assert_eq!(s.step_instruction().1, None);
            if s.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(&s.wram[0xf040..0xf042], &[0xc0, 0xde]);
        for (sample, expected) in case["expected"].as_array().unwrap().iter().enumerate() {
            let at = 0xf000 + sample * 2;
            let actual = u16::from_be_bytes([s.wram[at], s.wram[at + 1]]);
            assert_eq!(
                u64::from(actual),
                expected.as_u64().unwrap(),
                "case={} sample={sample}",
                case["name"]
            );
        }
    }
}
