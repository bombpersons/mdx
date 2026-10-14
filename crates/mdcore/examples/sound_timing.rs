//! Observe console instruction boundaries in a bounded master-clock window.
//! Usage: sound_timing ROM REPLAY FIRST_MASTER LAST_MASTER
//! Uses public state capture; never changes execution or private scheduler state.
use mdcore::{Cartridge, System};
use std::{fs, io::Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let replay: serde_json::Value = serde_json::from_slice(&fs::read(&args[2])?)?;
    let events = replay["events"].as_array().ok_or("missing events")?;
    let first: u64 = args[3].parse()?;
    let last: u64 = args[4].parse()?;
    let mut next = 0;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    while s.master_cycles() <= last {
        while let Some(e) = events
            .get(next)
            .filter(|e| e["frame"].as_u64().unwrap() <= s.vdp.frame)
        {
            let mut mask = 0;
            for b in e["buttons"].as_array().unwrap() {
                mask |= 1
                    << ["up", "down", "left", "right", "a", "b", "c", "start"]
                        .iter()
                        .position(|n| Some(*n) == b.as_str())
                        .ok_or("invalid button")?;
            }
            s.io.pad_buttons[e["port"].as_u64().unwrap_or(0) as usize] = mask;
            next += 1;
        }
        if s.master_cycles() >= first {
            let state = serde_json::to_value(s.save_state()?)?;
            writeln!(
                out,
                "{}",
                serde_json::json!({
                    "master": s.master_cycles(), "frame": s.vdp.frame, "pc": s.cpu.pc,
                    "z80": state["z80"], "z80_master": state["z80_master_clock"],
                    "dma_until": state["z80_dma_until"], "busreq": s.io.z80_bus_request,
                    "reset": s.io.z80_reset,
                })
            )?;
        }
        if let Some(e) = s.step_instruction().1 {
            return Err(format!("{e:?}").into());
        }
    }
    Ok(())
}
