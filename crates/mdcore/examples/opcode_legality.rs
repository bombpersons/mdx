//! Classify every opcode word as the CPU executes it in supervisor mode:
//! 0 illegal, 1 line A, 2 line F, 3 executed (or another exception).
use mdcore::cpu::m68000::{Exception, M68000};
use mdcore::cpu::{Bus, SparseBus};
fn main() -> std::io::Result<()> {
    let out = std::env::args().nth(1).expect("output path");
    let mut classes = vec![0u8; 65536];
    for op in 0..=0xFFFFu32 {
        let mut bus = SparseBus::new();
        bus.write16(0x1000, op as u16);
        let mut cpu = M68000::new(0x1000);
        cpu.a[7] = 0x8000;
        cpu.sr.0 = 0x2700;
        cpu.prime(&mut bus);
        let out = cpu.step(&mut bus);
        classes[op as usize] = match out.exception {
            Some(Exception::Illegal { .. }) => match op >> 12 {
                0xA => 1,
                0xF => 2,
                _ => 0,
            },
            _ => 3,
        };
    }
    std::fs::write(out, classes)
}
