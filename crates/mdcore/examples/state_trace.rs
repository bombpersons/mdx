//! Compact pre-instruction CPU state for event-aligned comparison with GPGX.
//! Run: cargo run --release -p mdcore --example state_trace -- ROM COUNT OUTPUT
use mdcore::{Cartridge, System};
use std::fs::File;
use std::io::{BufWriter, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(4..=5).contains(&args.len()) {
        return Err("expected ROM COUNT OUTPUT [START_PC_HEX]".into());
    }
    let start_pc = args.get(4).map(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16)).transpose()?;
    let mut started = start_pc.is_none();
    let mut skipped = 0u64;
    let count: u64 = args[2].parse()?;
    let mut system = System::new(Cartridge::load(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[3])?);
    let mut exceptions = 0;
    let mut recorded = 0;
    while recorded < count {
        if !started {
            started = Some(system.cpu.pc) == start_pc;
            if !started {
                if system.step_instruction().1.is_some() { exceptions += 1; }
                skipped += 1;
                if skipped >= 100_000_000 { return Err("start PC not reached within 100M steps".into()); }
                continue;
            }
        }
        // System handles interrupt entry as a separate step. GPGX's hook
        // observes instructions after entry, so do not emit a phantom event.
        if system.interrupt_due() {
            system.step_instruction();
            continue;
        }
        let cpu = &system.cpu;
        for value in [cpu.pc, cpu.sr.0 as u32].into_iter().chain(cpu.d).chain(cpu.a) {
            out.write_all(&value.to_le_bytes())?;
        }
        if system.step_instruction().1.is_some() {
            exceptions += 1;
        }
        recorded += 1;
    }
    out.flush()?;
    println!("{}", serde_json::json!({"rom_sha256":system.cart.sha256,
        "records":count,"bytes":count*72,"exceptions":exceptions,"skipped_steps":skipped,"start_pc":start_pc,
        "frame":system.vdp.frame,"final_pc":system.cpu.pc,"path":args[3]}));
    Ok(())
}
