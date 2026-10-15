//! Print interrupt state around a dispatch ordinal (debug).
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let target: u64 = args[2].parse()?;
    let mut dispatched = 0;
    while dispatched < target + 3 {
        let d = !s.interrupt_due() && !s.cpu.stopped;
        if dispatched + 300 >= target {
            println!("disp {dispatched} pos {} taken {} pc {:06x} sr {:04x} vint {} hint {} r1 {:02x} r0 {:02x} line {} due {}", s.vdp.master_clock() % 3420, s.vdp.vint_taken, s.cpu.pc, s.cpu.sr.0, s.vdp.vint_pending, s.vdp.hint_pending, s.vdp.registers[1], s.vdp.registers[0], s.vdp.line, s.interrupt_due());
        }
        s.step_instruction();
        if d { dispatched += 1; }
    }
    Ok(())
}
