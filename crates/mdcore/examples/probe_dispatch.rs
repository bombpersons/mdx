//! Run to a dispatch ordinal (interrupt entries excluded) and print VDP/CPU
//! state around the next few instructions. Usage: probe_dispatch ROM ORDINAL [COUNT]
use mdcore::{Cartridge, System};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let target: u64 = args[2].parse()?;
    let count: u64 = args.get(3).map(|c| c.parse()).transpose()?.unwrap_or(4);
    let mut dispatched = 0;
    while dispatched < target + count {
        let interrupt = s.interrupt_due();
        let dispatch = !interrupt && !s.cpu.stopped;
        if dispatched >= target {
            println!(
                "{} master {} frame {} line {} dot {} pc {:06X} sr {:04X} irq {:?} vint {} hint {} r0 {:02X} r1 {:02X} r10 {:02X} r12 {:02X} op {:04X} ff46 {:04X} dma {:02X}{:02X} r23 {:02X} busy {}",
                if dispatch { "dispatch" } else { "other" },
                s.master_cycles(), s.vdp.frame, s.vdp.line, s.vdp.dot, s.cpu.pc, s.cpu.sr.0,
                s.irq_level(), s.vdp.vint_pending, s.vdp.hint_pending,
                s.vdp.registers[0], s.vdp.registers[1], s.vdp.registers[10], s.vdp.registers[12],
                s.cpu.prefetch[0], u16::from_be_bytes([s.wram[0xFF46], s.wram[0xFF47]]), s.vdp.registers[20], s.vdp.registers[19], s.vdp.registers[23], s.vdp.dma_busy()
            );
        }
        s.step_instruction();
        if dispatch {
            dispatched += 1;
        }
    }
    Ok(())
}
