//! Print every VDP control-port word written in the first FRAMES frames.
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    s.vdp_control_log = Some(Vec::new());
    let frames: u64 = args[2].parse()?;
    while s.vdp.frame < frames { s.step_instruction(); }
    for (frame, pc, value, second) in s.vdp_control_log.unwrap() {
        println!("frame {frame} pc {pc:06X} {value:04X}{}", if second { " (second word)" } else { "" });
    }
    Ok(())
}
