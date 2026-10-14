//! Optional commercial replay at the first independently observed Z80 bus wait.
use mdcore::{Cartridge, System};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[test]
#[ignore = "external King's Bounty ROM/replay; independent bus timing checkpoint at frame943"]
fn first_z80_bus_request_stalls_the_68000_at_the_reference_clock() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cart = Cartridge::load(root.join("rom/original/kings_bounty.md")).unwrap();
    assert_eq!(
        cart.sha256,
        "5fd1494a6a47e33d85b22ebbc26a1156f31bb539cf54d4e109e1bcbddef8b4eb"
    );
    let replay_bytes = fs::read(root.join("testdata/kb_walk_replay.json")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&replay_bytes)),
        "1aeb82b91d912bd70e04c21c025beab1ea978111d38b6d5181f3e683e1a6b6df"
    );
    let replay: serde_json::Value = serde_json::from_slice(&replay_bytes).unwrap();
    let events = replay["events"].as_array().unwrap();
    let mut next_input = 0;
    let mut s = System::new(cart);
    s.observation_limit = 0;
    let mut dispatched = 0;
    for _ in 0..15000000 {
        while let Some(event) = events
            .get(next_input)
            .filter(|e| e["frame"].as_u64().unwrap() <= s.vdp.frame)
        {
            let mut mask = 0;
            for button in event["buttons"].as_array().unwrap() {
                mask |= 1
                    << ["up", "down", "left", "right", "a", "b", "c", "start"]
                        .iter()
                        .position(|name| Some(*name) == button.as_str())
                        .unwrap();
            }
            s.io.pad_buttons[event["port"].as_u64().unwrap_or(0) as usize] = mask;
            next_input += 1;
        }
        let interrupt = s.interrupt_due();
        if !interrupt && !s.cpu.stopped {
            if dispatched == 10184208 {
                assert_eq!(s.vdp.frame, 943);
                assert_eq!(s.cpu.pc, 0x133ac);
                // Observation-only original bus handler: CPU803765 ->803835,
                // Z80 PC0988, Z80 clocks803820 ->803865, in replay frame943.
                assert_eq!(s.master_cycles(), 943 * 896040 + 803835);
                return;
            }
            dispatched += 1;
        }
        assert_eq!(s.step_instruction().1, None);
    }
    panic!("reference dispatch target was not reached");
}
