//! Authored flat-bus instruction cases, paired with the external oracle harness.
use mdcore::{Z80Bus, Z80};
use std::io::{self, BufRead, Write};
struct Memory {
    bytes: Vec<u8>,
    writes: Vec<(u16, u8)>,
}
impl Z80Bus for Memory {
    fn read(&mut self, a: u16) -> u8 {
        self.bytes[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.bytes[a as usize] = v;
        self.writes.push((a, v));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let values: Vec<u32> = line?
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        let (kind, opcode, flags) = (values[0], values[1] as u8, values[2] as u8);
        let mut memory = Memory {
            bytes: (0u32..65536)
                .map(|i| ((i * 37) ^ (i >> 8) ^ 0x5a) as u8)
                .collect(),
            writes: Vec::new(),
        };
        let mut cpu = Z80::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x6806;
        cpu.a = 0xa5;
        cpu.f = flags;
        cpu.b = 0x13;
        cpu.c = 1;
        cpu.d = 0x24;
        cpu.e = 2;
        cpu.h = 0x35;
        cpu.l = 3;
        cpu.ix = 0x4604;
        cpu.iy = 0x5705;
        cpu.a_ = 0x98;
        cpu.f_ = 0x76;
        cpu.bc_ = 0x8765;
        cpu.de_ = 0x7654;
        cpu.hl_ = 0x6543;
        let mut code = Vec::new();
        if kind == 1 || kind == 5 {
            code.push(0xdd);
        }
        if kind == 2 || kind == 6 {
            code.push(0xfd);
        }
        if kind == 3 || kind == 5 || kind == 6 {
            code.push(0xcb);
        }
        if kind == 4 {
            code.push(0xed);
        }
        if kind >= 5 {
            code.push(6);
        }
        code.extend([opcode, 6, 0x80, 0]);
        memory.bytes[0x1000..0x1000 + code.len()].copy_from_slice(&code);
        let cycles = cpu.step(&mut memory);
        writeln!(
            out,
            "{}",
            serde_json::json!({"kind":kind,"opcode":opcode,"flags":flags,"cycles":cycles,
            "registers":[cpu.pc,cpu.af(),cpu.bc(),cpu.de(),cpu.hl(),cpu.ix,cpu.iy,cpu.sp,((cpu.a_ as u16)<<8)|cpu.f_ as u16,cpu.bc_,cpu.de_,cpu.hl_],"writes":memory.writes})
        )?;
    }
    Ok(())
}
