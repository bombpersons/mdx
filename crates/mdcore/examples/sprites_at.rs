//! List sprite entries (VRAM and internal cache) after FRAMES frames.
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let frames: u64 = args[2].parse()?;
    while s.vdp.frame < frames {
        s.step_instruction();
    }
    let h40 = s.vdp.registers[12] & 1 != 0;
    let satb = ((s.vdp.registers[5] as usize) << 9) & if h40 { 0xFC00 } else { 0xFE00 };
    let cache = s.vdp.raster.sat_cache();
    let w = |a: usize| u16::from_be_bytes([s.vdp.vram[a & 0xFFFF], s.vdp.vram[(a + 1) & 0xFFFF]]);
    let c = |a: usize| u16::from_be_bytes([cache[a & 0x3FF], cache[(a + 1) & 0x3FF]]);
    let mut link = 0usize;
    for _ in 0..80 {
        let e = satb + link * 8;
        println!("#{link:2} vram y {:3} sl {:04X} attr {:04X} x {:3} | cache y {:3} sl {:04X}",
            w(e) & 0x1FF, w(e + 2), w(e + 4), w(e + 6) & 0x1FF, c(link * 8) & 0x1FF, c(link * 8 + 2));
        link = (c(link * 8 + 2) & 0x7F) as usize;
        if link == 0 { break; }
    }
    Ok(())
}
