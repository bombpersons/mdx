use super::*;

/// A flat 64 KiB bus, so a test can lay out code and data explicitly.
struct Flat([u8; 0x1_0000]);

impl Flat {
    fn new() -> Self {
        Flat([0; 0x1_0000])
    }
    fn code(&mut self, at: u16, bytes: &[u8]) {
        self.0[at as usize..at as usize + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, at: u16, v: u16) {
        self.0[at as usize] = v as u8;
        self.0[at as usize + 1] = (v >> 8) as u8;
    }
    fn peek16(&self, at: u16) -> u16 {
        ((self.0[at as usize + 1] as u16) << 8) | self.0[at as usize] as u16
    }
}

impl Z80Bus for Flat {
    fn read(&mut self, addr: u16) -> u8 {
        self.0[addr as usize]
    }
    fn write(&mut self, addr: u16, value: u8) {
        self.0[addr as usize] = value;
    }
}

fn run(bus: &mut Flat, pc: u16, bytes: &[u8], steps: usize) -> Z80 {
    bus.code(pc, bytes);
    let mut cpu = Z80::new();
    cpu.pc = pc;
    cpu.sp = 0xFFFF;
    for _ in 0..steps {
        cpu.step(bus);
    }
    cpu
}

/// LD A,#2A / LD B,#0F / ADD A,B
#[test]
fn add_a_register_writes_the_result() {
    let mut bus = Flat::new();
    let cpu = run(&mut bus, 0x8000, &[0x3E, 0x2A, 0x06, 0x0F, 0x80], 3);
    assert_eq!(cpu.a, 0x39, "0x2A + 0x0F");
    assert_eq!(cpu.pc, 0x8005);
}

/// Carry out of a plain add, then consumed by the following ADC.
#[test]
fn carry_is_set_by_add_and_consumed_by_adc() {
    let mut bus = Flat::new();
    let cpu = run(&mut bus, 0x8000, &[0x3E, 0xFF, 0xC6, 0x01], 2);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flag(CF), "0xFF + 1 carries");
    assert!(cpu.flag(ZF));

    // 0x10 + 0xFF = 0x10F, so A is 0x0F with carry; ADC A,1 gives 0x11.
    let cpu = run(&mut bus, 0x8100, &[0x3E, 0x10, 0xC6, 0xFF, 0xCE, 0x01], 3);
    assert_eq!(cpu.a, 0x11, "0x0F + 1 + carry");
    assert!(
        !cpu.flag(CF),
        "0x0F + 1 + 1 = 0x11, so this one clears carry"
    );
}

#[test]
fn inc_preserves_carry_but_cpl_does_not_clear_it() {
    let mut bus = Flat::new();
    // SCF then INC A. INC must leave carry alone.
    let cpu = run(&mut bus, 0x8000, &[0x37, 0x3C], 2);
    assert_eq!(cpu.a, 0x01);
    assert!(cpu.flag(CF), "INC must not clear carry");
    // CPL complements the accumulator and must also leave carry alone.
    let cpu = run(&mut bus, 0x8000, &[0x37, 0x3E, 0x00, 0x2F], 3);
    assert_eq!(cpu.a, 0xFF);
    assert!(cpu.flag(CF));
    assert!(cpu.flag(NF), "CPL sets the subtract flag");
}

/// A relative jump is measured from the address after the two-byte instruction.
/// Getting this off by one is the classic silent branch bug.
#[test]
fn jr_displacement_is_relative_to_the_end_of_the_instruction() {
    let mut bus = Flat::new();
    let cpu = run(&mut bus, 0x8000, &[0x18, 0x02, 0x00, 0x00], 1);
    assert_eq!(cpu.pc, 0x8004, "PC after the fetch is 0x8002, plus 2");
}

/// The counted-delay shape drivers use. LD B,3 / DEC B / JR NZ,-2 must take the
/// branch three times and not the fourth.
#[test]
fn djnz_counts_b_down_and_falls_through_on_zero() {
    let mut bus = Flat::new();
    // The branch is -3, because the displacement is relative to the address
    // after the two-byte instruction (0x8005), landing back on DEC B.
    let cpu = run(&mut bus, 0x8000, &[0x06, 0x03, 0x05, 0x20, 0xFD], 7);
    assert_eq!(cpu.b, 0);
    assert_eq!(cpu.pc, 0x8005, "three taken branches, then one untaken");
}

/// BIT 7,(HL) must test the byte at HL, not the address. H=0x90 L=0x00, and
/// the address 0x9000 has bit 7 clear while the byte stored there does not.
#[test]
fn bit_on_memory_tests_the_byte_not_the_address() {
    let mut bus = Flat::new();
    bus.0[0x9000] = 0b1000_0000;
    let cpu = run(&mut bus, 0x8000, &[0x26, 0x90, 0x2E, 0x00, 0xCB, 0x7E], 3);
    assert!(!cpu.flag(ZF), "bit 7 of the byte at 0x9000 is set");
}

/// BIT 7,(HL) is two bytes. If a displacement were consumed here the program
/// counter would desynchronise from the instruction stream.
#[test]
fn cb_prefixed_memory_forms_consume_no_displacement_without_a_prefix() {
    let mut bus = Flat::new();
    bus.0[0x9000] = 0x01;
    // BIT 0,(HL) then INC A. BIT is 2 bytes, so INC A is at 0x8006.
    let cpu = run(
        &mut bus,
        0x8000,
        &[0x26, 0x90, 0x2E, 0x00, 0xCB, 0x46, 0x3C],
        4,
    );
    assert_eq!(cpu.a, 1, "INC A ran, so BIT consumed exactly two bytes");
    assert_eq!(cpu.pc, 0x8007);
}

/// DD 66 01 is LD H,(IX+1): the displacement is read once, so PC lands past it.
/// IX itself must not move - only the indexed inc/dec and push/pop change it.
#[test]
fn dd_prefix_reaches_through_ix_and_consumes_one_displacement() {
    let mut bus = Flat::new();
    bus.0[0x0005] = 0x7E;
    let cpu = run(
        &mut bus,
        0x8000,
        &[0xDD, 0x21, 0x05, 0x00, 0xDD, 0x66, 0x00],
        2,
    );
    assert_eq!(
        cpu.pc, 0x8007,
        "opcode, prefix, displacement word, prefix, op, disp"
    );
    assert_eq!(cpu.h, 0x7E);
    assert_eq!(cpu.ix, 0x0005, "LD H,(IX+d) leaves IX alone");
}

/// LDIR from 0x9000 to 0x9100, two bytes, in two passes.
#[test]
fn ldir_copies_until_bc_reaches_zero_and_clears_parity_on_the_last_pass() {
    let mut bus = Flat::new();
    bus.code(0x9000, &[0x0A, 0x0B, 0x11, 0x22]);
    let cpu = run(
        &mut bus,
        0x8000,
        &[
            0x21, 0x00, 0x90, 0x11, 0x00, 0x91, 0x01, 0x02, 0x00, 0xED, 0xB0,
        ],
        5,
    );
    assert_eq!(bus.peek16(0x9100), 0x0B0A, "two bytes copied, source order");
    assert_eq!(bus.0[0x9101], 0x0B);
    assert_eq!(bus.0[0x9102], 0, "nothing past the count is touched");
    assert_eq!(cpu.bc(), 0);
    assert_eq!(cpu.hl(), 0x9002);
    assert_eq!(cpu.de(), 0x9102);
    assert!(!cpu.flag(PF), "PF is cleared on the pass that empties BC");
}

#[test]
fn cpir_stops_on_the_match_and_reports_it() {
    let mut bus = Flat::new();
    bus.code(0x9000, &[0x11, 0x22, 0x33, 0x11]);
    let cpu = run(
        &mut bus,
        0x8000,
        &[0x21, 0x00, 0x90, 0x01, 0x04, 0x00, 0x3E, 0x33, 0xED, 0xB1],
        6,
    );
    assert_eq!(cpu.hl(), 0x9003, "HL stops on the match, not past it");
    assert!(cpu.flag(ZF), "CP sets Z when the bytes are equal");
    assert_eq!(cpu.bc(), 1, "BC is decremented once per comparison");
}

#[test]
fn call_and_return_round_trip_through_the_stack() {
    let mut bus = Flat::new();
    let cpu = run(&mut bus, 0x8000, &[0xCD, 0x06, 0x80], 1);
    assert_eq!(cpu.pc, 0x8006);
    bus.code(0x8006, &[0xC9]);
    let cpu = run(&mut bus, 0x8000, &[0xCD, 0x06, 0x80], 2);
    assert_eq!(cpu.pc, 0x8003, "RET returns past the CALL");
    assert_eq!(cpu.sp, 0xFFFF, "balanced, so SP is back where it started");
}

#[test]
fn push_and_pop_round_trip_every_pair() {
    let mut bus = Flat::new();
    let cpu = run(&mut bus, 0x8000, &[0x21, 0x34, 0x12, 0xE5, 0xC1], 3);
    assert_eq!(cpu.bc(), 0x1234);
    assert_eq!(cpu.sp, 0xFFFF);
}

/// Block I/O is how the drivers drive the YM2612 and PSG, so the port address
/// has to come from BC and the data from (HL).
#[test]
fn otir_writes_every_byte_to_the_port_in_bc() {
    struct Ports {
        flat: Flat,
        written: Vec<(u16, u8)>,
    }
    impl Z80Bus for Ports {
        fn read(&mut self, a: u16) -> u8 {
            self.flat.read(a)
        }
        fn write(&mut self, a: u16, v: u8) {
            self.flat.write(a, v)
        }
        fn port_out(&mut self, port: u16, v: u8) {
            self.written.push((port, v));
        }
    }
    let mut bus = Ports {
        flat: Flat::new(),
        written: Vec::new(),
    };
    // LD HL,9000 / LD B,2 / LD C,0 / OTIR. B carries the count and C the port,
    // and OTIR decrements B alone - so the port address moves 0x0200, 0x0100
    // while C stays put. (Setting B to zero instead would wrap it to 0xFF on
    // the first pass, which is real hardware behaviour and a classic reason
    // drivers avoid the block forms for single ports.)
    bus.flat.code(
        0x8000,
        &[0x21, 0x00, 0x90, 0x06, 0x02, 0x0E, 0x00, 0xED, 0xB3],
    );
    bus.flat.code(0x9000, &[0xAB, 0xCD]);
    let mut cpu = Z80::new();
    cpu.pc = 0x8000;
    for _ in 0..5 {
        cpu.step(&mut bus);
    }
    assert_eq!(bus.written, vec![(0x0200, 0xAB), (0x0100, 0xCD)]);
    assert_eq!(cpu.b, 0, "B counts the bytes down");
    assert_eq!(cpu.c, 0, "C, which holds the port, is untouched");
}

#[test]
fn halt_parks_the_cpu_and_an_interrupt_releases_it() {
    struct Irq {
        flat: Flat,
        line: bool,
    }
    impl Z80Bus for Irq {
        fn read(&mut self, a: u16) -> u8 {
            self.flat.read(a)
        }
        fn write(&mut self, a: u16, v: u8) {
            self.flat.write(a, v)
        }
    }
    let mut bus = Irq {
        flat: Flat::new(),
        line: false,
    };
    bus.flat.code(0x8000, &[0xFB, 0xED, 0x5E, 0x76]); // EI / IM 2 / HALT
    let mut cpu = Z80::new();
    cpu.pc = 0x8000;
    cpu.i = 0x00;
    for _ in 0..3 {
        cpu.step(&mut bus);
    }
    assert!(cpu.halted, "HALT parks rather than skipping");
    cpu.i = 0x00;
    cpu.irq_pending = true;
    bus.flat.word(0x00FF, 0x8050); // IM 2 vector at I:0xFF
    let _ = bus.line;
    cpu.step(&mut bus);
    assert!(!cpu.halted, "an accepted interrupt leaves HALT");
    assert_eq!(cpu.pc, 0x8050);
    assert!(!cpu.iff1, "EI is cancelled once the interrupt is taken");
}

#[test]
fn reset_holds_the_cpu_until_the_line_releases() {
    struct Reset {
        flat: Flat,
        held: bool,
        steps: u32,
    }
    impl Z80Bus for Reset {
        fn read(&mut self, a: u16) -> u8 {
            self.flat.read(a)
        }
        fn write(&mut self, a: u16, v: u8) {
            self.flat.write(a, v)
        }
        fn in_reset(&self) -> bool {
            self.held
        }
    }
    let mut bus = Reset {
        flat: Flat::new(),
        held: true,
        steps: 0,
    };
    bus.flat.code(0x8000, &[0x3C]); // INC A
    let mut cpu = Z80::new();
    cpu.pc = 0x8000;
    for _ in 0..100 {
        if let Some(n) = cpu.step(&mut bus) {
            bus.steps += n as u32;
        }
    }
    assert_eq!(
        bus.steps, 0,
        "no T-states retire while the line is asserted"
    );
    assert_eq!(cpu.a, 0);
    bus.held = false;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 1, "and it runs as soon as it is released");
}
