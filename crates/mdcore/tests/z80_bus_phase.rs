//! Authored two-CPU programs; expected HV samples come from unchanged GPGX.
//! Includes local RAM/FM controls, shared ROM/PSG access, DMA and frame wrap.
use mdcore::{Cartridge, System};

#[test]
fn sound_bus_and_dma_phases_match_independent_counter_samples() {
    verify(include_str!("fixtures/z80_bus_hv.json"), 224);
}

#[test]
fn delayed_sound_accesses_around_dma_update_boundaries_match_oracle() {
    verify(include_str!("fixtures/z80_dma_overshoot_hv.json"), 98);
}

#[test]
fn repeating_shared_bus_moves_with_ignored_index_prefix_match_oracle() {
    verify(include_str!("fixtures/z80_block_move_hv.json"), 28);
}

fn verify(raw: &str, count: usize) {
    let fixture: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), count);
    for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        for (at, word) in case["program_words"].as_array().unwrap().iter().enumerate() {
            rom[0x200 + at * 2..0x202 + at * 2]
                .copy_from_slice(&(word.as_u64().unwrap() as u16).to_be_bytes());
        }
        let mut s = System::new(Cartridge::from_bytes("sound-bus.md", rom).unwrap());
        s.observation_limit = 0;
        for _ in 0..100000 {
            assert_eq!(s.step_instruction().1, None, "case {index}");
            if s.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(
            &s.wram[0xf040..0xf042],
            &[0xc0, 0xde],
            "case {index} incomplete"
        );
        for (sample, expected) in case["expected"].as_array().unwrap().iter().enumerate() {
            let at = 0xf000 + sample * 2;
            let actual = u16::from_be_bytes([s.wram[at], s.wram[at + 1]]);
            assert_eq!(
                u64::from(actual),
                expected.as_u64().unwrap(),
                "case {index} sample {sample}: kind={} wide={} nops={} DMA={}",
                case["kind"],
                case["wide"],
                case["nops"],
                case["dma_words"]
            );
        }
    }
}
