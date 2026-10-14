use mdcore::cpu::bus::{Bus, SparseBus};
use mdcore::cpu::m68000::{Exception, M68000};
fn main() {
    let mut illegal = Vec::new();
    for op in 0..=0xFFFFu32 {
        let mut bus = SparseBus::new();
        bus.write16(0x1000, op as u16);
        let mut cpu = M68000::new(0x1000);
        cpu.a = [0x2000; 8];
        cpu.prime(&mut bus);
        if let Some(Exception::Illegal { .. }) = cpu.step(&mut bus).exception {
            illegal.push(op);
        }
    }
    println!("{}", illegal.iter().map(|o| o.to_string()).collect::<Vec<_>>().join(","));
}
