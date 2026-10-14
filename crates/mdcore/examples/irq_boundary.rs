//! Observe instruction dispatch and interrupt entry in an authored ROM.
//! Uses the console clock directly, including fractional CPU-cycle bus waits.
use mdcore::{Cartridge, System};
use std::{collections::VecDeque, io::Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: irq_boundary ROM")?;
    let mut s = System::new(Cartridge::load(path)?);
    s.observation_limit = 0;
    let mut before = VecDeque::new();
    let mut remaining: u32 = 0;
    let mut interrupts = 0;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for step in 0..100000 {
        let irq = s.irq_level().filter(|level| s.cpu.sr.accepts(*level));
        let mut row = serde_json::json!({"step":step,"master":s.master_cycles(),
            "frame":s.vdp.frame,"line":s.vdp.line,"dot":s.vdp.dot,
            "pc":s.cpu.pc,"sr":s.cpu.sr.0,"d":s.cpu.d,"a":s.cpu.a,"irq":irq});
        let (cycles, exception) = s.step_instruction();
        assert_eq!(exception, None);
        row["cycles"] = cycles.into();
        row["next_pc"] = s.cpu.pc.into();
        row["post_d"] = serde_json::json!(s.cpu.d);
        if irq.is_some() {
            interrupts += 1;
            for previous in before.drain(..) {
                writeln!(out, "{previous}")?;
            }
            remaining = 9;
        }
        if step < 20 || remaining > 0 {
            writeln!(out, "{row}")?;
            remaining = remaining.saturating_sub(1);
        } else {
            before.push_back(row);
            if before.len() > 8 {
                before.pop_front();
            }
        }
        if interrupts == 2 && remaining == 0 {
            break;
        }
    }
    Ok(())
}
