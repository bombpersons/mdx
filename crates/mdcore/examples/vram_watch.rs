//! Report the instruction whose step changes VRAM[ADDR] during frames FIRST..=LAST.
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let addr = usize::from_str_radix(args[2].trim_start_matches("0x"), 16)?;
    let (first, last): (u64, u64) = (args[3].parse()?, args[4].parse()?);
    while s.vdp.frame < first { s.step_instruction(); }
    let mut before = s.vdp.vram[addr..addr + 16].to_vec();
    while s.vdp.frame <= last {
        let (pc, line, dot, code, latch) = (s.cpu.pc, s.vdp.line, s.vdp.dot, s.vdp.code, s.vdp.address_latch);
        let regs = s.vdp.registers;
        s.step_instruction();
        if s.vdp.vram[addr..addr + 16] != before[..] {
            println!("frame {} line {line} dot {dot} pc {pc:06X} code {code:02X} latch {latch:04X} r1 {:02X} r15 {:02X} len {:02X}{:02X} src {:02X}{:02X}{:02X} queue {} -> {:02X?}",
                s.vdp.frame, regs[1], regs[15], regs[20], regs[19], regs[23], regs[22], regs[21], s.vdp.dma_queue.len(), &s.vdp.vram[addr..addr + 8]);
            before = s.vdp.vram[addr..addr + 16].to_vec();
        }
    }
    Ok(())
}
