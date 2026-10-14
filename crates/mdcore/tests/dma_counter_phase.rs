//! Authored transfers, sampled by unchanged GPGX. No commercial assets needed.
use mdcore::{Cartridge, System};

fn verify_transfers(known_gap: bool) {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dma_hv_samples.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        if case["known_gap"].as_bool().unwrap() != known_gap {
            continue;
        }
        let wide = case["wide"].as_bool().unwrap();
        let display = case["display"].as_bool().unwrap();
        let words = case["words"].as_u64().unwrap() as u16;
        let target = case["target"].as_str().unwrap();
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        let mut code = vec![0x33fc, 0x8104, 0x00c0, 0x0004];
        if wide {
            code.extend([0x33fc, 0x8c81, 0x00c0, 0x0004]);
        }
        for (register, value) in [
            (1, if display { 0x54 } else { 0x14 }),
            (15, 2),
            (19, words & 255),
            (20, words >> 8),
            (21, 0),
            (22, 8),
            (23, 0),
        ] {
            code.extend([0x33fc, 0x8000 | register << 8 | value, 0x00c0, 0x0004]);
        }
        let (high, low) = match target {
            "vram" => (0x4000, 0x0080),
            "cram" => (0xc000, 0x0080),
            "vsram" => (0x4000, 0x0090),
            _ => panic!("unknown DMA target"),
        };
        code.extend([0x23fc, high, low, 0x00c0, 0x0004]);
        for sample in 0..32 {
            code.extend([0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000 + sample * 2]);
            code.extend([0x4e71; 13]);
        }
        code.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
        for (i, word) in code.iter().enumerate() {
            rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
        }
        let mut system = System::new(Cartridge::from_bytes("dma-phase.md", rom).unwrap());
        for _ in 0..10000 {
            let measure = wide && !display && words == 256 && target == "vram"
                && system.cpu.prefetch[0] == 0x23fc;
            let before = system.master_cycles();
            let outcome = system.step_instruction();
            assert_eq!(outcome.1, None);
            if measure {
                // Independent original-oracle dispatch785736 ->794345.
                assert_eq!(before, 785736);
                assert_eq!(system.master_cycles() - before, 8609);
                assert_eq!(outcome.0, 1230); // rounded CPU cost is8610 clocks
            }
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
                "wide={wide} display={display} words={words} target={target} sample={sample}"
            );
        }
    }
}

#[test]
fn dma_resume_phases_match_independent_counter_samples() {
    verify_transfers(false);
}

#[test]
fn large_active_vram_dma_across_initial_frame_matches_oracle() {
    verify_transfers(true);
}
