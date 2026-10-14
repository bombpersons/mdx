//! Run to a dispatch ordinal, then report how long VDP DMA busy stays set.
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let target: u64 = args[2].parse()?;
    let mut dispatched = 0;
    while dispatched < target {
        let d = !s.interrupt_due() && !s.cpu.stopped;
        s.step_instruction();
        if d { dispatched += 1; }
    }
    let start = s.master_cycles();
    println!("start master {start} frame {} line {} dot {} busy {} r1 {:02X} r12 {:02X} len {:02X}{:02X}", s.vdp.frame, s.vdp.line, s.vdp.dot, s.vdp.dma_busy(), s.vdp.registers[1], s.vdp.registers[12], s.vdp.registers[20], s.vdp.registers[19]);
    let mut last_line = s.vdp.line;
    let mut changes = 0;
    while s.vdp.dma_busy() {
        s.step_instruction();
        if s.vdp.registers[1] != 0x74 && changes < 5 { println!("r1 changed {:02X} at frame {} line {}", s.vdp.registers[1], s.vdp.frame, s.vdp.line); changes += 1; }
        if s.vdp.line == 200 && last_line != 200 { println!("line 200 vblank {} frame {}", s.vdp.vblank, s.vdp.frame); }
        last_line = s.vdp.line;
    }
    println!("busy for {} master, end frame {} line {}", s.master_cycles() - start, s.vdp.frame, last_line);
    Ok(())
}
