//! Z80 instruction trace (HALT idling excluded) to compare with the oracle's
//! MDX_ORACLE_Z80_TRACE output. Usage: z80_trace ROM FRAMES OUTPUT [MAX]
//! Lines: frame master pc af bc de hl ix iy sp (decimal, space separated).
use mdcore::{Cartridge, System};
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let frames: u64 = args[2].parse()?;
    let max: usize = args.get(4).map(|m| m.parse()).transpose()?.unwrap_or(5_000_000);
    s.z80_trace = Some(mdcore::observe::Z80Trace { pc_range: None, max_records: max, observed: 0, records: Vec::new() });
    while s.vdp.frame < frames {
        s.step_instruction();
    }
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3])?);
    for r in &s.z80_trace.as_ref().unwrap().records {
        writeln!(out, "{} {} {} {} {} {} {} {} {} {} {:02x}{:02x}{:02x}{:02x}", r.frame, r.master_cycle, r.pc, r.af, r.bc, r.de, r.hl, r.ix, r.iy, r.sp, r.bytes[0], r.bytes[1], r.bytes[2], r.bytes[3])?;
    }
    Ok(())
}
