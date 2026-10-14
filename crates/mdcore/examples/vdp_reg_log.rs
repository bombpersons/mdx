//! Print VDP register changes (with line/dot) during a frame window.
//! Usage: vdp_reg_log ROM FIRST_FRAME LAST_FRAME
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let (first, last): (u64, u64) = (args[2].parse()?, args[3].parse()?);
    while s.vdp.frame < first {
        s.step_instruction();
    }
    let mut before = s.vdp.registers;
    let mut sat = s.vdp.vram.clone();
    let mut cram = s.vdp.cram.clone();
    let mut vsram = s.vdp.vsram.clone();
    while s.vdp.frame <= last {
        let pc = s.cpu.pc;
        let (line, dot) = (s.vdp.line, s.vdp.dot);
        s.step_instruction();
        for (i, (a, b)) in before.iter().zip(s.vdp.registers.iter()).enumerate() {
            if a != b {
                println!("frame {} line {line} dot {dot} pc {pc:06X} reg {i} {a:02X}->{b:02X}", s.vdp.frame);
            }
        }
        let satb = ((s.vdp.registers[5] as usize) << 9) & 0xFC00;
        if s.vdp.vram[satb..satb + 0x280] != sat[satb..satb + 0x280] {
            println!("frame {} line {line} dot {dot} pc {pc:06X} SAT changed", s.vdp.frame);
        }
        if s.vdp.cram != cram {
            let i: Vec<_> = (0..64).filter(|i| s.vdp.cram[i * 2..i * 2 + 2] != cram[i * 2..i * 2 + 2]).collect();
            println!("frame {} line {line} dot {dot} pc {pc:06X} CRAM {:?}", s.vdp.frame, i);
        }
        if s.vdp.vsram != vsram {
            println!("frame {} line {line} dot {dot} pc {pc:06X} VSRAM {:02X?}", s.vdp.frame, &s.vdp.vsram[..4]);
        }
        cram = s.vdp.cram.clone();
        vsram = s.vdp.vsram.clone();
        sat = s.vdp.vram.clone();
        before = s.vdp.registers;
    }
    Ok(())
}
