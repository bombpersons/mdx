//! Print cartridge mapping facts and words at addresses (debug).
use mdcore::{Cartridge, System};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let s = System::new(Cartridge::load(&args[1])?);
    println!("sram start {:x} end {:x} mapped {} len {}", s.cart_hw.sram_start, s.cart_hw.sram_end, s.cart_hw.sram_mapped, s.cart_hw.sram.len());
    println!("period {:x} len {:x} data214 {:02x?}", s.cart.mirror_period(), s.cart.data.len(), &s.cart.data[0x214..0x21a]);
    println!("open-bus banks e: {} realtec {}", s.cart_hw.open_bus_bank(0xE), s.cart_hw.hw.realtec);
    for a in &args[2..] {
        let a = u32::from_str_radix(a, 16)?;
        println!("{a:06x}: {:?} {:?}", s.cart_hw.read(a), s.cart_hw.read(a + 1));
    }
    Ok(())
}
