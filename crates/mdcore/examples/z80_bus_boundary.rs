//! Observe simple Z80 absolute bus operations between console step boundaries.
//! Not a sub-instruction bus trace: reports only unambiguous single-instruction
//! 32/3A transitions, with both surrounding console clocks and CPU PCs.
use mdcore::{Cartridge, System};
use std::{fs, io::Write};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let replay: serde_json::Value = serde_json::from_slice(&fs::read(&args[2])?)?;
    let events = replay["events"].as_array().unwrap();
    let mut next = 0;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut count = 0;
    for step in 0..100000000u64 {
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
                        .unwrap();
            }
            s.io.pad_buttons[e["port"].as_u64().unwrap_or(0) as usize] = mask;
            next += 1;
        }
        let z = s.z80;
        let pc = s.cpu.pc;
        let clock = s.master_cycles();
        let op = s.z80_ram[z.pc as usize & 8191];
        let a = u16::from(s.z80_ram[z.pc.wrapping_add(1) as usize & 8191])
            | u16::from(s.z80_ram[z.pc.wrapping_add(2) as usize & 8191]) << 8;
        let (_, exception) = s.step_instruction();
        if let Some(e) = exception {
            return Err(format!("{e:?}").into());
        }
        if matches!(op, 0x32 | 0x3a)
            && (a >= 0x8000 || a >> 8 == 0x7f)
            && s.z80.pc == z.pc.wrapping_add(3)
            && matches!(s.z80.cycles - z.cycles, 13 | 16)
        {
            writeln!(
                out,
                "{}",
                serde_json::json!({"step":step,"frame":s.vdp.frame,"master_before":clock,"master_after":s.master_cycles(),"cpu_before":pc,"cpu_after":s.cpu.pc,"z80_pc":z.pc,"address":a,"write":op==0x32,"value":z.a,"z80_tstates":z.cycles})
            )?;
            count += 1;
            if count == 16 {
                break;
            }
        }
    }
    Ok(())
}
