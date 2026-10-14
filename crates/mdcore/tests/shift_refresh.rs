//! Independent HV readings expose register-shift refresh scheduling differences.
//! Samples were executed by unchanged GPGX; no commercial cartridge is needed.
use mdcore::{Cartridge, System};

#[test]
fn shift_and_rotate_refresh_phases_match_independent_counter_samples() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shift_refresh_samples.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let opcode = case["opcode"].as_u64().unwrap() as u16;
        let count = case["count"].as_u64().unwrap() as u16;
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        let mut code = vec![
            0x33fc,
            0x8104,
            0x00c0,
            0x0004, // Mode5
            0x33fc,
            0x8c81,
            0x00c0,
            0x0004, // H40
            0x7400 | count,
            0x76ff, // count D2, data D3
        ];
        for sample in 0..32 {
            code.extend([
                opcode,
                0x3039,
                0x00c0,
                0x0008, // shift; read HV into D0
                0x33c0,
                0x00ff,
                0xf000 + sample * 2, // store sample
            ]);
            code.extend([0x4e71; 13]);
        }
        code.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
        for (i, word) in code.iter().enumerate() {
            rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
        let mut system = System::new(Cartridge::from_bytes("shift-phase.md", rom).unwrap());
        for _ in 0..10000 {
            assert_eq!(system.step_instruction().1, None);
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(&system.wram[0xf040..0xf042], &[0xc0, 0xde]);
        for (sample, expected) in case["expected"].as_array().unwrap().iter().enumerate() {
            let at = 0xf000 + sample * 2;
            let actual = u16::from_be_bytes([system.wram[at], system.wram[at + 1]]);
            assert_eq!(
                u64::from(actual),
                expected.as_u64().unwrap(),
                "opcode={opcode:04x} count={count} sample={sample}"
            );
        }
    }
}
