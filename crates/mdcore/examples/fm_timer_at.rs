//! FM timer state synced to each frame edge FIRST..=LAST.
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let (first, last): (u64, u64) = (args[2].parse()?, args[3].parse()?);
    for f in first..=last {
        while s.vdp.frame < f { s.step_instruction(); }
        let mut fm = s.fm.clone();
        fm.sync_to(f * s.vdp.frame_master());
        println!("frame {f} {:?}", fm.timer_debug());
    }
    Ok(())
}
