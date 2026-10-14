//! Independent authored transfers across the initial viewport latch.
use mdcore::{Cartridge, System};

#[test]
fn large_transfers_latch_viewport_height_at_each_frame_edge() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dma_viewport_hv.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 96);
    for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let mut rom = vec![0; 65536];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        for (at, word) in case["program_words"].as_array().unwrap().iter().enumerate() {
            rom[0x200 + at * 2..0x202 + at * 2]
                .copy_from_slice(&(word.as_u64().unwrap() as u16).to_be_bytes());
        }
        let mut s = System::new(Cartridge::from_bytes("viewport-dma.md", rom).unwrap());
        s.observation_limit = 0;
        for _ in 0..10000 {
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
                "case {index} sample {sample}: height={} wide={} words={} target={}",
                case["height"],
                case["wide"],
                case["words"],
                case["target"]
            );
        }
    }
}
