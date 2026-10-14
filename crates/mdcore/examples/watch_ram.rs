//! Observe changes to a folded work-RAM range at instruction boundaries.
//! ROM FRAMES START_HEX END_HEX OUTPUT [REPLAY_JSON] [FROM_FRAME]. End is exclusive. This records net
//! changes, not bus writes: same-value writes and changes reverted in one
//! instruction are unobserved. The ring retains the latest 64 changes.
use mdcore::{Cartridge, System};
use mdcore::input::Button;
use serde::Deserialize;
use std::{collections::VecDeque, fs};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event { frame: u64, #[serde(default)] port: usize, buttons: Vec<Button> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replay { events: Vec<Event> }

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(6..=8).contains(&args.len()) { return Err("expected ROM FRAMES START_HEX END_HEX OUTPUT [REPLAY_JSON] [FROM_FRAME]".into()); }
    let frames: u64 = args[2].parse()?;
    let start = usize::from_str_radix(args[3].trim_start_matches("0x"), 16)?;
    let end = usize::from_str_radix(args[4].trim_start_matches("0x"), 16)?;
    if start >= end || end > 65536 { return Err("invalid folded work-RAM range".into()); }
    let from_frame: u64 = args.get(7).map(|s| s.parse()).transpose()?.unwrap_or(0);
    if from_frame >= frames { return Err("FROM_FRAME must be before FRAMES".into()); }
    let replay_bytes = args.get(6).map(fs::read).transpose()?;
    let events = replay_bytes.as_ref().map(|b| serde_json::from_slice::<Replay>(b))
        .transpose()?.map(|r| r.events).unwrap_or_default();
    let mut previous = None;
    let mut ports = 0u8;
    for event in &events {
        if event.port > 1 || previous.is_some_and(|f| event.frame < f) { return Err("invalid replay port or event order".into()); }
        if previous != Some(event.frame) { ports = 0; }
        if ports & (1 << event.port) != 0 { return Err("duplicate replay frame/port".into()); }
        ports |= 1 << event.port;
        previous = Some(event.frame);
    }
    let mut next_input = 0;
    let mut system = System::new(Cartridge::load(&args[1])?);
    let mut before = system.wram[start..end].to_vec();
    let mut changes = VecDeque::new();
    let mut total = 0u64;
    let mut steps = 0u64;
    while system.vdp.frame < frames && steps < 100_000_000 {
        while next_input < events.len() && events[next_input].frame <= system.vdp.frame {
            let event = &events[next_input];
            system.io.pad_buttons[event.port] = event.buttons.iter().fold(0, |m, b| m | b.mask());
            next_input += 1;
        }
        let pc = system.cpu.pc;
        let registers = (system.cpu.d, system.cpu.a, system.cpu.sr.0);
        if let Some(e) = system.step_instruction().1 { return Err(format!("exception at {pc:#X}: {e:?}").into()); }
        steps += 1;
        let after = &system.wram[start..end];
        if after != before && system.vdp.frame >= from_frame {
            total += 1;
            if changes.len() == 64 { changes.pop_front(); }
            changes.push_back(serde_json::json!({"pc":format!("0x{pc:08X}"),
                "step":steps,"frame":system.vdp.frame,"before":before,"after":after,
                "d":registers.0,"a":registers.1,"sr":registers.2}));
            before.copy_from_slice(after);
        }
        if system.vdp.frame < from_frame { before.copy_from_slice(after); }
    }
    let report = serde_json::json!({"rom_sha256":system.cart.sha256,
        "frames":system.vdp.frame,"requested_frames":frames,"steps":steps,"range":[start,end],
        "total_changes":total,"retained_changes":changes,"dropped_changes":total.saturating_sub(64),
        "from_frame":from_frame,"input_sha256":replay_bytes.as_ref().map(|b| {
            use sha2::{Digest, Sha256}; format!("{:x}", Sha256::digest(b))
        }),"input_events_applied":next_input,"input_events_pending":events.len()-next_input,
        "limitations":["instruction-boundary net changes only; not a bus-write trace"]});
    fs::write(&args[5], serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::json!({"path":args[5],"total_changes":total,"frame":system.vdp.frame}));
    Ok(())
}
