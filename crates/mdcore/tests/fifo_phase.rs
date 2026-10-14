//! Reference FIFO-full waits cover all seven NTSC frame/CPU clock alignments.
use mdcore::{Cartridge, System};

#[test]
fn fifo_wait_rounding_matches_frame_relative_reference_phases() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/fifo_hv_samples.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut code: Vec<u16> = vec![0x33fc, 0x8104, 0x00c0, 0x0004];
        if case["wide"].as_bool().unwrap() {
            code.extend([0x33fc, 0x8c81, 0x00c0, 0x0004]);
        }
        let mut loops = (case["frame"].as_u64().unwrap() * 896040 + 138 * 3420 - 784462) / 70;
        while loops != 0 {
            let chunk = loops.min(65536);
            code.extend([0x3c3c, (chunk - 1) as u16, 0x51ce, 0xfffe]);
            loops -= chunk;
        }
        let (high, low) = match case["target"].as_str().unwrap() {
            "vram" => (0x4000, 0),
            "cram" => (0xc000, 0),
            "vsram" => (0x4000, 0x10),
            _ => unreachable!(),
        };
        code.extend([
            0x33fc, 0x8144, 0x00c0, 0x0004, 0x23fc, high, low, 0x00c0, 0x0004,
        ]);
        for sample in 0..32u16 {
            for _ in 0..8 {
                code.extend([0x33fc, 0, 0x00c0, 0]);
            }
            code.extend([0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000 + sample * 2]);
            code.extend([0x4e71; 13]);
        }
        code.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        for (i, word) in code.iter().enumerate() {
            rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
        let mut s = System::new(Cartridge::from_bytes("fifo-phase.md", rom).unwrap());
        for _ in 0..200000 {
            assert_eq!(s.step_instruction().1, None);
            if s.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(&s.wram[0xf040..0xf042], &[0xc0, 0xde]);
        assert!(s.vdp.fifo_stall_master_cycles > 0, "{case}");
        for (sample, expected) in case["expected"].as_array().unwrap().iter().enumerate() {
            let at = 0xf000 + sample * 2;
            let actual = u16::from_be_bytes([s.wram[at], s.wram[at + 1]]);
            assert_eq!(
                u64::from(actual),
                expected.as_u64().unwrap(),
                "sample={sample}: {case}"
            );
        }
    }
}
