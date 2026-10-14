//! Z80 disassembler for sound drivers, table-driven from the opcode's
//! x/y/z/p/q fields. Covers the unprefixed, CB, ED, DD/FD and DDCB/FDCB
//! groups, including undocumented IXH/IXL forms, `sll`, `in (c)`/`out (c),0`
//! and indexed bit operations that also store to a register.
//! Output is lowercase with 0x-prefixed hex, like the 68000 listing.

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insn {
    pub length: u8,
    pub text: String,
    /// False for undefined ED opcodes and redundant prefixes, which the CPU
    /// executes as no-ops (or ignores) rather than as a documented instruction.
    pub decoded: bool,
}

const R: [&str; 8] = ["b", "c", "d", "e", "h", "l", "(hl)", "a"];
const RP: [&str; 4] = ["bc", "de", "hl", "sp"];
const RP2: [&str; 4] = ["bc", "de", "hl", "af"];
const CC: [&str; 8] = ["nz", "z", "nc", "c", "po", "pe", "p", "m"];
const ALU: [&str; 8] = ["add a,", "adc a,", "sub ", "sbc a,", "and ", "xor ", "or ", "cp "];
const ROT: [&str; 8] = ["rlc", "rrc", "rl", "rr", "sla", "sra", "sll", "srl"];
const IM: [&str; 8] = ["0", "0", "1", "2", "0", "0", "1", "2"];
const BLOCK: [[&str; 4]; 4] = [
    ["ldi", "cpi", "ini", "outi"],
    ["ldd", "cpd", "ind", "outd"],
    ["ldir", "cpir", "inir", "otir"],
    ["lddr", "cpdr", "indr", "otdr"],
];

/// Index register context for DD (ix) / FD (iy) prefixed instructions.
#[derive(Clone, Copy)]
struct Index {
    name: &'static str,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn byte(&mut self) -> u8 {
        let b = self.bytes.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        b
    }
    fn word(&mut self) -> u16 {
        u16::from_le_bytes([self.byte(), self.byte()])
    }
}

fn hex8(v: u8) -> String {
    format!("0x{v:02X}")
}
fn hex16(v: u16) -> String {
    format!("0x{v:04X}")
}
fn displacement(d: u8) -> String {
    let d = d as i8;
    if d < 0 {
        format!("-0x{:02X}", -(d as i16))
    } else {
        format!("+0x{d:02X}")
    }
}

/// Decode one instruction from `bytes` (at least 4 bytes when available)
/// located at Z80 address `pc`. Missing trailing bytes read as zero.
pub fn disassemble(bytes: &[u8], pc: u16) -> Insn {
    let mut c = Cursor { bytes, at: 0 };
    let op = c.byte();
    let (text, decoded) = match op {
        0xCB => (cb(&mut c), true),
        0xED => ed(&mut c),
        0xDD | 0xFD => {
            let index = Index { name: if op == 0xDD { "ix" } else { "iy" } };
            match bytes.get(1).copied().unwrap_or(0) {
                // A following prefix makes this one a no-op.
                0xDD | 0xED | 0xFD => return Insn { length: 1, text: "nop (prefix ignored)".into(), decoded: false },
                0xCB => {
                    c.byte();
                    (indexed_cb(&mut c, index), true)
                }
                _ => indexed(&mut c, pc, index),
            }
        }
        _ => {
            c.at = 0;
            (main(&mut c, pc, None), true)
        }
    };
    Insn { length: c.at as u8, text, decoded }
}

/// Register name with optional index substitution. `memory` holds the
/// already-formatted (ix+d) operand when the instruction uses one.
fn reg(r: usize, index: Option<Index>, memory: &Option<String>) -> String {
    match (r, index, memory) {
        (6, None, _) => "(hl)".into(),
        (6, Some(_), Some(m)) => m.clone(),
        // H/L stay H/L when the same instruction also uses (ix+d).
        (4, Some(ix), None) => format!("{}h", ix.name),
        (5, Some(ix), None) => format!("{}l", ix.name),
        _ => R[r].into(),
    }
}

/// Unprefixed decode; with `index`, HL forms become IX/IY forms.
fn main(c: &mut Cursor, pc: u16, index: Option<Index>) -> String {
    let start = c.at;
    let op = c.byte();
    let (x, y, z) = (op >> 6, ((op >> 3) & 7) as usize, (op & 7) as usize);
    let (p, q) = (y >> 1, y & 1);
    let hl = index.map_or("hl", |ix| ix.name);
    let rp = |p: usize| if p == 2 { hl.to_string() } else { RP[p].to_string() };
    let rp2 = |p: usize| if p == 2 { hl.to_string() } else { RP2[p].to_string() };
    // Instructions using (hl) as memory take a displacement under a prefix.
    let uses_memory = (x == 1 && (y == 6 || z == 6) && !(y == 6 && z == 6))
        || (x == 2 && z == 6)
        || (x == 0 && (4..=6).contains(&z) && y == 6);
    let memory = match index {
        Some(ix) if uses_memory => Some(format!("({}{})", ix.name, displacement(c.byte()))),
        _ => None,
    };
    let r = |r: usize| reg(r, index, &memory);
    let relative = |c: &mut Cursor| {
        let d = c.byte() as i8;
        hex16(pc.wrapping_add((c.at - start) as u16).wrapping_add(d as u16))
    };
    match x {
        0 => match z {
            0 => match y {
                0 => "nop".into(),
                1 => "ex af,af'".into(),
                2 => format!("djnz {}", relative(c)),
                3 => format!("jr {}", relative(c)),
                _ => format!("jr {},{}", CC[y - 4], relative(c)),
            },
            1 if q == 0 => format!("ld {},{}", rp(p), hex16(c.word())),
            1 => format!("add {hl},{}", rp(p)),
            2 => match (q, p) {
                (0, 0) => "ld (bc),a".into(),
                (0, 1) => "ld (de),a".into(),
                (0, 2) => format!("ld ({}),{hl}", hex16(c.word())),
                (0, _) => format!("ld ({}),a", hex16(c.word())),
                (_, 0) => "ld a,(bc)".into(),
                (_, 1) => "ld a,(de)".into(),
                (_, 2) => format!("ld {hl},({})", hex16(c.word())),
                _ => format!("ld a,({})", hex16(c.word())),
            },
            3 => format!("{} {}", if q == 0 { "inc" } else { "dec" }, rp(p)),
            4 => format!("inc {}", r(y)),
            5 => format!("dec {}", r(y)),
            6 => {
                let target = r(y);
                format!("ld {target},{}", hex8(c.byte()))
            }
            _ => ["rlca", "rrca", "rla", "rra", "daa", "cpl", "scf", "ccf"][y].into(),
        },
        1 if y == 6 && z == 6 => "halt".into(),
        1 => {
            // With (ix+d), the other operand keeps plain h/l.
            let plain = |n: usize| if memory.is_some() && n != 6 { R[n].to_string() } else { r(n) };
            format!("ld {},{}", plain(y), plain(z))
        }
        2 => format!("{}{}", ALU[y], r(z)),
        _ => match z {
            0 => format!("ret {}", CC[y]),
            1 if q == 0 => format!("pop {}", rp2(p)),
            1 => match p {
                0 => "ret".into(),
                1 => "exx".into(),
                2 => format!("jp ({hl})"),
                _ => format!("ld sp,{hl}"),
            },
            2 => format!("jp {},{}", CC[y], hex16(c.word())),
            3 => match y {
                0 => format!("jp {}", hex16(c.word())),
                2 => format!("out ({}),a", hex8(c.byte())),
                3 => format!("in a,({})", hex8(c.byte())),
                4 => format!("ex (sp),{hl}"),
                5 => "ex de,hl".into(),
                6 => "di".into(),
                7 => "ei".into(),
                _ => unreachable!("CB handled by caller"),
            },
            4 => format!("call {},{}", CC[y], hex16(c.word())),
            5 if q == 0 => format!("push {}", rp2(p)),
            5 => format!("call {}", hex16(c.word())),
            6 => format!("{}{}", ALU[y], hex8(c.byte())),
            _ => format!("rst {}", hex8((y * 8) as u8)),
        },
    }
}

fn cb_text(op: u8, operand: &str) -> String {
    let (x, y) = (op >> 6, (op >> 3) & 7);
    match x {
        0 => format!("{} {operand}", ROT[usize::from(y)]),
        1 => format!("bit {y},{operand}"),
        2 => format!("res {y},{operand}"),
        _ => format!("set {y},{operand}"),
    }
}

fn cb(c: &mut Cursor) -> String {
    let op = c.byte();
    cb_text(op, R[usize::from(op & 7)])
}

/// DDCB/FDCB: displacement precedes the opcode. Non-BIT forms with z != 6
/// also store the result in a register (undocumented).
fn indexed_cb(c: &mut Cursor, index: Index) -> String {
    let memory = format!("({}{})", index.name, displacement(c.byte()));
    let op = c.byte();
    let z = usize::from(op & 7);
    let text = cb_text(op, &memory);
    if op >> 6 != 1 && z != 6 {
        format!("{text},{}", R[z])
    } else {
        text
    }
}

fn indexed(c: &mut Cursor, pc: u16, index: Index) -> (String, bool) {
    let op = c.bytes.get(1).copied().unwrap_or(0);
    let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
    // Opcodes that name HL, H, L or (HL): those the prefix redirects.
    let touches_hl = match x {
        0 => match z {
            1 => y == 4 || y & 1 == 1,
            2 | 3 => y == 4 || y == 5,
            4..=6 => (4..=6).contains(&y),
            _ => false,
        },
        1 => op != 0x76 && ((4..=6).contains(&y) || (4..=6).contains(&z)),
        2 => (4..=6).contains(&z),
        _ => matches!(op, 0xE1 | 0xE3 | 0xE5 | 0xE9 | 0xF9),
    };
    c.at = 1;
    if touches_hl {
        (main(c, pc, Some(index)), true)
    } else {
        // The prefix has no effect on this opcode; it executes unprefixed.
        let text = main(c, pc.wrapping_add(1), None);
        (format!("{text} (prefix ignored)"), false)
    }
}

fn ed(c: &mut Cursor) -> (String, bool) {
    let op = c.byte();
    let (x, y, z) = (op >> 6, usize::from((op >> 3) & 7), op & 7);
    let (p, q) = (y >> 1, y & 1);
    let text = match (x, z) {
        (1, 0) if y == 6 => "in (c)".into(),
        (1, 0) => format!("in {},(c)", R[y]),
        (1, 1) if y == 6 => "out (c),0".into(),
        (1, 1) => format!("out (c),{}", R[y]),
        (1, 2) => format!("{} hl,{}", if q == 0 { "sbc" } else { "adc" }, RP[p]),
        (1, 3) if q == 0 => format!("ld ({}),{}", hex16(c.word()), RP[p]),
        (1, 3) => format!("ld {},({})", RP[p], hex16(c.word())),
        (1, 4) => "neg".into(),
        (1, 5) => if y == 1 { "reti" } else { "retn" }.into(),
        (1, 6) => format!("im {}", IM[y]),
        (1, 7) => ["ld i,a", "ld r,a", "ld a,i", "ld a,r", "rrd", "rld", "nop", "nop"][y].into(),
        (2, 0..=3) if y >= 4 => BLOCK[y - 4][usize::from(z)].into(),
        _ => return (format!("nop (undefined 0xED{op:02X})"), false),
    };
    let documented = !(x == 1 && z == 7 && y >= 6);
    (text, documented)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: &[u8], pc: u16) -> (u8, String) {
        let insn = disassemble(bytes, pc);
        (insn.length, insn.text)
    }

    #[test]
    fn representative_encodings_decode_to_zilog_mnemonics() {
        let cases: &[(&[u8], u16, u8, &str)] = &[
            (&[0x00], 0, 1, "nop"),
            (&[0xF3], 0, 1, "di"),
            (&[0xED, 0x56], 0, 2, "im 1"),
            (&[0x31, 0x00, 0x20], 0, 3, "ld sp,0x2000"),
            (&[0x3A, 0x0A, 0x1C], 0, 3, "ld a,(0x1C0A)"),
            (&[0x18, 0xFE], 0x100, 2, "jr 0x0100"),
            (&[0x10, 0x05], 0x100, 2, "djnz 0x0107"),
            (&[0x20, 0x80], 0x200, 2, "jr nz,0x0182"),
            (&[0x76], 0, 1, "halt"),
            (&[0x7E], 0, 1, "ld a,(hl)"),
            (&[0xCB, 0x7E], 0, 2, "bit 7,(hl)"),
            (&[0xCB, 0x37], 0, 2, "sll a"),
            (&[0xED, 0xB0], 0, 2, "ldir"),
            (&[0xED, 0x4B, 0x34, 0x12], 0, 4, "ld bc,(0x1234)"),
            (&[0xED, 0x70], 0, 2, "in (c)"),
            (&[0xDD, 0x21, 0x00, 0x1C], 0, 4, "ld ix,0x1C00"),
            (&[0xDD, 0x7E, 0x05], 0, 3, "ld a,(ix+0x05)"),
            (&[0xFD, 0x36, 0xFE, 0x20], 0, 4, "ld (iy-0x02),0x20"),
            (&[0xDD, 0x66, 0x01], 0, 3, "ld h,(ix+0x01)"),
            (&[0xDD, 0x74, 0x01], 0, 3, "ld (ix+0x01),h"),
            (&[0xDD, 0x64], 0, 2, "ld ixh,ixh"),
            (&[0xFD, 0x7D], 0, 2, "ld a,iyl"),
            (&[0xDD, 0x86, 0x10], 0, 3, "add a,(ix+0x10)"),
            (&[0xDD, 0x34, 0x03], 0, 3, "inc (ix+0x03)"),
            (&[0xDD, 0xE9], 0, 2, "jp (ix)"),
            (&[0xDD, 0xCB, 0x02, 0x46], 0, 4, "bit 0,(ix+0x02)"),
            (&[0xFD, 0xCB, 0x02, 0xC0], 0, 4, "set 0,(iy+0x02),b"),
            (&[0xDD, 0x09], 0, 2, "add ix,bc"),
            (&[0xC7], 0, 1, "rst 0x00"),
            (&[0xFF], 0, 1, "rst 0x38"),
            (&[0xE3], 0, 1, "ex (sp),hl"),
            (&[0x08], 0, 1, "ex af,af'"),
            (&[0xD3, 0x7F], 0, 2, "out (0x7F),a"),
        ];
        for (bytes, pc, length, expected) in cases {
            assert_eq!(text(bytes, *pc), (*length, expected.to_string()), "{bytes:02X?}");
        }
        assert!(!disassemble(&[0xED, 0x00], 0).decoded);
        assert!(!disassemble(&[0xDD, 0x00], 0).decoded);
        assert_eq!(disassemble(&[0xDD, 0xDD, 0x00], 0).length, 1);
    }
}
