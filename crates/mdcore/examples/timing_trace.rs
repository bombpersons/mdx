//! Observation-only compact instruction trace for independent timing comparisons.
//! Usage: timing_trace ROM DISPATCH_LIMIT OUTPUT [REPLAY] [SKIP_DISPATCHES]
//! Records: LE u64 exact master clock, then 18 LE u32 PC,SR,D0..D7,A0..A7.
//! Interrupt-entry and STOP-idle steps do not dispatch instructions.
use mdcore::{Cartridge, System};
use std::{
    fs,
    io::{BufWriter, Write},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let limit: u64 = args[2].parse()?;
    let skip: u64 = args.get(5).map(|x| x.parse()).transpose()?.unwrap_or(0);
    let target = skip.checked_add(limit).ok_or("dispatch count overflow")?;
    let mut output = BufWriter::new(fs::File::create(&args[3])?);
    let replay: serde_json::Value = if let Some(path) = args.get(4) {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        serde_json::json!({"events": []})
    };
    let events = replay["events"]
        .as_array()
        .ok_or("replay requires events array")?;
    let mut next_input = 0;
    let mut dispatched = 0;
    let mut steps = 0;
    while dispatched < target && steps < target.saturating_mul(2).max(100000) {
        while let Some(event) = events
            .get(next_input)
            .filter(|e| e["frame"].as_u64().is_some_and(|f| f <= s.vdp.frame))
        {
            let mut mask = 0;
            for button in event["buttons"]
                .as_array()
                .ok_or("buttons must be an array")?
            {
                let bit = ["up", "down", "left", "right", "a", "b", "c", "start"]
                    .iter()
                    .position(|name| Some(*name) == button.as_str())
                    .ok_or("unknown button")?;
                mask |= 1 << bit;
            }
            let port = event["port"].as_u64().unwrap_or(0) as usize;
            *s.io.pad_buttons.get_mut(port).ok_or("invalid port")? = mask;
            next_input += 1;
        }
        let interrupt = s.interrupt_due();
        if !interrupt && !s.cpu.stopped {
            if dispatched >= skip {
                output.write_all(&s.master_cycles().to_le_bytes())?;
                for value in [s.cpu.pc, u32::from(s.cpu.sr.0)]
                    .into_iter()
                    .chain(s.cpu.d)
                    .chain(s.cpu.a)
                {
                    output.write_all(&value.to_le_bytes())?;
                }
            }
            dispatched += 1;
        }
        // Exceptions (TRAP, CHK, ...) are taken by the system like the
        // reference; the trace simply continues at the handler.
        s.step_instruction();
        steps += 1;
    }
    if dispatched != target {
        return Err("dispatch target was not reached".into());
    }
    eprintln!(
        "{dispatched} dispatches, {steps} system steps, frame {}",
        s.vdp.frame
    );
    Ok(())
}
