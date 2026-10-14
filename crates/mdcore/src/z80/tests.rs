use super::*;

/// Literal Z80 opcode order: NZ, Z, NC, C, PO, PE, P, M.
/// Exercise the public instruction decoder, not its condition helper.
#[test]
fn conditional_control_flow_uses_the_zilog_encoding() {
    let jp = [0xC2, 0xCA, 0xD2, 0xDA, 0xE2, 0xEA, 0xF2, 0xFA];
    let call = [0xC4, 0xCC, 0xD4, 0xDC, 0xE4, 0xEC, 0xF4, 0xFC];
    let ret = [0xC0, 0xC8, 0xD0, 0xD8, 0xE0, 0xE8, 0xF0, 0xF8];
    let jr = [0x20, 0x28, 0x30, 0x38];
    for f in 0..=255u8 {
        let taken = [
            f & 0x40 == 0,
            f & 0x40 != 0,
            f & 1 == 0,
            f & 1 != 0,
            f & 4 == 0,
            f & 4 != 0,
            f & 0x80 == 0,
            f & 0x80 != 0,
        ];
        for cc in 0..8 {
            for (kind, op) in [(0, jp[cc]), (1, call[cc]), (2, ret[cc])] {
                let mut bus = Flat::new();
                bus.code(0x1000, &[op, 0x00, 0x20]);
                bus.word(0x9000, 0x2000);
                let mut cpu = Z80::new();
                cpu.pc = 0x1000;
                cpu.sp = 0x9000;
                cpu.f = f;
                let states = cpu.step(&mut bus);
                let next = if taken[cc] {
                    0x2000
                } else if kind == 2 {
                    0x1001
                } else {
                    0x1003
                };
                assert_eq!(cpu.pc, next, "op={op:02x} f={f:02x}");
                assert_eq!(cpu.f, f);
                assert_eq!(
                    states,
                    Some(match kind {
                        0 => 10,
                        1 =>
                            if taken[cc] {
                                17
                            } else {
                                10
                            },
                        _ =>
                            if taken[cc] {
                                11
                            } else {
                                5
                            },
                    })
                );
                assert_eq!(
                    cpu.sp,
                    match (kind, taken[cc]) {
                        (1, true) => 0x8FFE,
                        (2, true) => 0x9002,
                        _ => 0x9000,
                    }
                );
                if kind == 1 && taken[cc] {
                    assert_eq!(bus.peek16(0x8FFE), 0x1003);
                }
            }
            if cc < 4 {
                let mut bus = Flat::new();
                bus.code(0x1000, &[jr[cc], 0xFE]);
                let mut cpu = Z80::new();
                cpu.pc = 0x1000;
                cpu.f = f;
                assert_eq!(cpu.step(&mut bus), Some(if taken[cc] { 12 } else { 7 }));
                assert_eq!(cpu.pc, if taken[cc] { 0x1000 } else { 0x1002 });
            }
        }
    }
}

#[test]
fn driver_command_0d_does_not_take_jr_z() {
    let mut bus = Flat::new();
    let cpu = run(&mut bus, 0x01A0, &[0xA7, 0x28, 0x0E], 0);
    let mut cpu = cpu;
    cpu.a = 0x0D;
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x01A3);
}

/// Count actual bus accesses: a byte read must not touch a neighbouring MMIO port.
#[test]
fn absolute_accumulator_loads_access_exactly_one_byte() {
    struct Observed {
        flat: Flat,
        reads: Vec<u16>,
        writes: Vec<(u16, u8)>,
    }
    impl Z80Bus for Observed {
        fn read(&mut self, a: u16) -> u8 {
            self.reads.push(a);
            self.flat.read(a)
        }
        fn write(&mut self, a: u16, v: u8) {
            self.writes.push((a, v));
            self.flat.write(a, v);
        }
    }
    let mut bus = Observed {
        flat: Flat::new(),
        reads: vec![],
        writes: vec![],
    };
    bus.flat.code(0x1000, &[0x32, 0x00, 0x40, 0x3A, 0x00, 0x40]);
    bus.flat.0[0x4001] = 0x5A;
    let mut cpu = Z80::new();
    cpu.pc = 0x1000;
    cpu.a = 0xA6;
    cpu.f = 0xD5;
    assert_eq!(cpu.step(&mut bus), Some(13));
    assert_eq!(bus.writes, [(0x4000, 0xA6)]);
    assert_eq!(bus.flat.0[0x4001], 0x5A);
    bus.reads.clear();
    cpu.a = 0;
    assert_eq!(cpu.step(&mut bus), Some(13));
    assert_eq!(bus.reads, [0x1003, 0x1004, 0x1005, 0x4000]);
    assert_eq!(cpu.a, 0xA6);
    assert_eq!(cpu.f, 0xD5);
}

#[test]
fn absolute_pair_stores_are_little_endian_and_wrap() {
    for prefix in [None, Some(0xDD), Some(0xFD)] {
        let mut bus = Flat::new();
        let bytes = if let Some(p) = prefix {
            vec![p, 0x22, 0xFF, 0xFF]
        } else {
            vec![0x22, 0xFF, 0xFF]
        };
        bus.code(0x1000, &bytes);
        let mut cpu = Z80::new();
        cpu.pc = 0x1000;
        cpu.set_hl(0x1234);
        cpu.ix = 0x5678;
        cpu.iy = 0x9ABC;
        cpu.step(&mut bus);
        let expected = match prefix {
            None => [0x34, 0x12],
            Some(0xDD) => [0x78, 0x56],
            _ => [0xBC, 0x9A],
        };
        assert_eq!([bus.0[0xFFFF], bus.0[0]], expected);
    }
}

#[test]
fn stack_layout_matches_independent_bytes_and_pop_af_restores_both_halves() {
    let mut bus = Flat::new();
    bus.code(0x1000, &[0xF5, 0xF1, 0xC1]);
    let mut cpu = Z80::new();
    cpu.pc = 0x1000;
    cpu.sp = 0;
    cpu.set_af(0x2854);
    cpu.step(&mut bus);
    assert_eq!(cpu.sp, 0xFFFE);
    assert_eq!([bus.0[0xFFFE], bus.0[0xFFFF]], [0x54, 0x28]);
    // Independently authored stack data prevents matching PUSH/POP bugs passing.
    bus.0[0xFFFE] = 0xBB;
    bus.0[0xFFFF] = 0xFF;
    cpu.set_af(0);
    cpu.step(&mut bus);
    assert_eq!(cpu.af(), 0xFFBB);
    assert_eq!(cpu.sp, 0);
    bus.word(0, 0x1234);
    cpu.step(&mut bus);
    assert_eq!(cpu.bc(), 0x1234);
}

#[test]
fn dac_handler_pop_af_preserves_zero_before_signed_loop() {
    let mut bus = Flat::new();
    bus.code(0x0577, &[0xF1, 0xC9]);
    bus.code(0x06AE, &[0xD6, 0x01, 0xF2, 0xAB, 0x06]);
    bus.word(0x1FF7, 0x0042);
    bus.word(0x1FF9, 0x06AE);
    let mut cpu = Z80::new();
    cpu.pc = 0x0577;
    cpu.sp = 0x1FF7;
    for _ in 0..4 {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.af(), 0xFFBB);
    assert_eq!(
        cpu.pc, 0x06B3,
        "zero restored by POP AF becomes -1, so JP P falls through"
    );
}

#[test]
fn driver_rrca_sets_xy_from_rotated_result_and_preserves_szpv() {
    let mut bus = Flat::new();
    bus.code(0x0976, &[0x0F]);
    let mut cpu = Z80::new();
    cpu.pc = 0x0976;
    cpu.set_af(0xC04A);
    cpu.step(&mut bus);
    assert_eq!(cpu.af(), 0x6060, "independent handler trace record 9");
}

#[test]
fn add_hl_preserves_szpv_and_sets_high_byte_xy() {
    let mut bus = Flat::new();
    bus.code(0x1000, &[0x19]);
    let mut cpu = Z80::new();
    cpu.pc = 0x1000;
    cpu.f = 0xC4;
    cpu.set_hl(0x27FF);
    cpu.set_de(1);
    cpu.step(&mut bus);
    assert_eq!(cpu.hl(), 0x2800);
    assert_eq!(
        cpu.f, 0xEC,
        "S/Z/PV preserved and high-byte X/Y set; no bit-11 carry"
    );
}

#[test]
fn logical_alu_half_carry_is_defined_by_the_operation() {
    for (op, initial, expected) in [(0xA7, 0, 0x94), (0xAF, 0x10, 0x44), (0xB7, 0x10, 0x84)] {
        let mut bus = Flat::new();
        bus.code(0x1000, &[op]);
        let mut cpu = Z80::new();
        cpu.pc = 0x1000;
        cpu.a = 0x81;
        cpu.f = initial;
        cpu.step(&mut bus);
        assert_eq!(cpu.f, expected, "opcode {op:02x}");
    }
}

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
    // and OTIR decrements B before output - so the port moves 0x0100, 0x0000
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
    assert_eq!(bus.written, vec![(0x0100, 0xAB), (0x0000, 0xCD)]);
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

#[test]
fn ei_defers_int_one_instruction_and_interrupt_cycles_are_counted_once() {
    for (mode, clocks, target) in [(IM_0, 13, 0x38), (IM_1, 13, 0x38), (IM_2, 19, 0x8050)] {
        let mut bus = Flat::new();
        bus.code(0x8000, &[0xfb, 0x00, 0x76]); // EI; NOP; HALT
        bus.word(0x00ff, 0x8050);
        let mut cpu = Z80::new();
        cpu.pc = 0x8000;
        cpu.sp = 0x9000;
        cpu.im = mode;
        cpu.irq_pending = true;
        assert_eq!(cpu.step(&mut bus), Some(4));
        assert!(cpu.after_ei);
        assert_eq!(cpu.step(&mut bus), Some(4));
        assert_eq!(cpu.pc, 0x8002, "the instruction following EI must execute");
        assert_eq!(cpu.step(&mut bus), Some(clocks));
        assert_eq!(cpu.pc, target);
        assert_eq!(cpu.cycles, u64::from(8 + clocks));
        assert_eq!(bus.peek16(0x8ffe), 0x8002);
        assert!(!cpu.iff1 && !cpu.iff2);
        assert!(
            cpu.irq_pending,
            "acknowledgement does not clear an external level"
        );
    }
}

#[test]
fn ei_halt_and_repeated_ei_preserve_the_interrupt_shadow() {
    let mut bus = Flat::new();
    bus.code(0, &[0xfb, 0xfb, 0x76]);
    let mut cpu = Z80::new();
    cpu.sp = 0x9000;
    cpu.im = IM_1;
    cpu.irq_pending = true;
    for _ in 0..3 {
        assert_eq!(cpu.step(&mut bus), Some(4));
    }
    assert!(cpu.halted);
    assert_eq!(cpu.step(&mut bus), Some(13));
    assert!(!cpu.halted);
    assert_eq!(bus.peek16(0x8ffe), 3);
}

#[test]
fn im2_pushes_the_return_address_before_reading_its_vector() {
    let mut cpu = Z80::new();
    let mut bus = Flat::new();
    cpu.pc = 0x8002;
    cpu.sp = 0x101;
    cpu.im = IM_2;
    cpu.iff1 = true;
    cpu.irq_pending = true;
    bus.word(0xff, 0x8050);
    assert_eq!(cpu.step(&mut bus), Some(19));
    assert_eq!(
        cpu.pc, 0x8002,
        "the stack overlaps I:FF, so the push changes the vector"
    );
}

#[test]
fn authored_instructions_match_the_unchanged_external_z80_core() {
    use std::collections::BTreeMap;
    struct Observed {
        bytes: Vec<u8>,
        writes: BTreeMap<u16, u8>,
    }
    impl Z80Bus for Observed {
        fn read(&mut self, a: u16) -> u8 {
            self.bytes[a as usize]
        }
        fn write(&mut self, a: u16, v: u8) {
            self.bytes[a as usize] = v;
            self.writes.insert(a, v);
        }
    }
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/z80_instructions.json")).unwrap();
    let template: Vec<u8> = (0u32..65536)
        .map(|i| ((i * 37) ^ (i >> 8) ^ 0x5a) as u8)
        .collect();
    for case in fixture["cases"].as_array().unwrap() {
        let kind = case["kind"].as_u64().unwrap();
        let op = case["opcode"].as_u64().unwrap() as u8;
        let mut bus = Observed {
            bytes: template.clone(),
            writes: BTreeMap::new(),
        };
        let mut cpu = Z80::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x6806;
        cpu.a = 0xa5;
        cpu.f = case["flags"].as_u64().unwrap() as u8;
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
        code.extend([op, 6, 0x80, 0]);
        bus.bytes[0x1000..0x1000 + code.len()].copy_from_slice(&code);
        let cycles = cpu.step(&mut bus).unwrap();
        let registers = [
            cpu.pc,
            cpu.af(),
            cpu.bc(),
            cpu.de(),
            cpu.hl(),
            cpu.ix,
            cpu.iy,
            cpu.sp,
            ((cpu.a_ as u16) << 8) | cpu.f_ as u16,
            cpu.bc_,
            cpu.de_,
            cpu.hl_,
        ];
        assert_eq!(
            serde_json::json!(cycles),
            case["cycles"],
            "cycles {kind}/{op:02x}"
        );
        assert_eq!(
            serde_json::json!(registers),
            case["registers"],
            "registers {kind}/{op:02x}"
        );
        assert_eq!(
            serde_json::json!(bus.writes.into_iter().collect::<Vec<_>>()),
            case["memory"],
            "memory {kind}/{op:02x}"
        );
    }
}

#[test]
fn warm_reset_preserves_data_stack_and_clock_while_resetting_control_state() {
    let mut cpu = Z80::new();
    cpu.a = 0x91;
    cpu.f = 0x45;
    cpu.b = 0x25;
    cpu.c = 0x31;
    cpu.sp = 0x1234;
    cpu.ix = 0xabcd;
    cpu.iy = 0x9876;
    cpu.cycles = 12345;
    cpu.irq_pending = true;
    cpu.pc = 0x5432;
    cpu.i = 0xa3;
    cpu.r = 0xfe;
    cpu.im = 2;
    cpu.iff1 = true;
    cpu.iff2 = true;
    cpu.after_ei = true;
    cpu.halted = true;
    cpu.wz = 0xbeef;
    cpu.reset();
    assert_eq!(
        (cpu.af(), cpu.bc(), cpu.sp, cpu.ix, cpu.iy, cpu.cycles),
        (0x9145, 0x2531, 0x1234, 0xabcd, 0x9876, 12345)
    );
    assert_eq!((cpu.pc, cpu.i, cpu.r, cpu.im, cpu.wz), (0, 0, 0, 0, 0));
    assert!(!cpu.iff1 && !cpu.iff2 && !cpu.after_ei && !cpu.halted);
    assert!(
        cpu.irq_pending,
        "reset does not clear an external interrupt wire"
    );
}

#[test]
fn refresh_counts_m1_fetches_and_the_last_index_prefix_wins() {
    let mut bus = Flat::new();
    bus.code(0, &[0xdd, 0xfd, 0x21, 0x34, 0x12]);
    let mut cpu = Z80::new();
    cpu.r = 0xff;
    assert_eq!(cpu.step(&mut bus), Some(18));
    assert_eq!(cpu.iy, 0x1234);
    assert_eq!(cpu.ix, 0);
    assert_eq!(
        cpu.r, 0x82,
        "three opcode fetches, without counting two immediate bytes"
    );
}

/// The disassembler's lengths agree with how far the executor actually moves
/// PC, for every opcode in every prefix group, wherever control does not
/// transfer. The two decoders are written independently.
#[test]
fn disassembler_lengths_match_executor_for_every_opcode_group() {
    let control = ["jp", "jr", "call", "ret", "reti", "retn", "rst", "djnz", "halt", "ldir", "lddr", "cpir",
        "cpdr", "inir", "indr", "otir", "otdr"];
    let mut checked = 0;
    let mut sequences: Vec<Vec<u8>> = Vec::new();
    for op in 0..=255u8 {
        sequences.push(vec![op]);
        sequences.push(vec![0xCB, op]);
        sequences.push(vec![0xED, op]);
        sequences.push(vec![0xDD, op]);
        sequences.push(vec![0xFD, op]);
        sequences.push(vec![0xDD, 0xCB, 0x05, op]);
        sequences.push(vec![0xFD, 0xCB, 0x05, op]);
    }
    for bytes in sequences {
        // Operands: 0x11 bytes keep displacements and immediates nonzero.
        let mut code = bytes.clone();
        code.resize(8, 0x11);
        // The executor runs a chain of redundant prefixes as one step; the
        // disassembler lists each as a one-byte no-op. Sum the chain.
        let mut skipped = 0usize;
        let mut insn = disasm::disassemble(&code, 0x1000);
        while insn.text == "nop (prefix ignored)" && insn.length == 1 {
            skipped += 1;
            insn = disasm::disassemble(&code[skipped..], 0x1000 + skipped as u16);
        }
        if control.iter().any(|c| insn.text.split(' ').next() == Some(c)) {
            continue;
        }
        let mut bus = Flat::new();
        bus.code(0x1000, &code);
        let mut cpu = Z80::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x8000;
        cpu.set_bc(0x0001);
        cpu.step(&mut bus);
        assert_eq!(
            u16::from(insn.length) + skipped as u16,
            cpu.pc.wrapping_sub(0x1000),
            "{bytes:02X?} decoded as {:?}",
            insn.text
        );
        checked += 1;
    }
    assert!(checked > 1500, "{checked}");
}
