//! Independent decimal arithmetic and 68000 operand-layout regressions.
use mdcore::{Bus, SparseBus, M68000};

fn run(opcode: u16, src: u8, dst: u8, flags: u16) -> M68000 {
    let mut bus = SparseBus::new();
    bus.write16(0x200, opcode);
    let mut cpu = M68000::new(0x200);
    cpu.d[1] = 0x12345600 | u32::from(src);
    cpu.d[0] = 0xabcdef00 | u32::from(dst);
    cpu.sr.0 = 0x2700 | flags;
    cpu.prime(&mut bus);
    let result = cpu.step(&mut bus);
    assert_eq!(result.exception, None);
    assert_eq!(result.cycles, 6);
    assert_eq!(cpu.pc, 0x202);
    assert_eq!(cpu.d[1], 0x12345600 | u32::from(src));
    assert_eq!(cpu.d[0] & !0xff, 0xabcdef00);
    cpu
}

#[test]
fn hokuto_decimal_timer_instruction_is_subtract_not_or() {
    let cpu = run(0x8101, 1, 0x99, 4);
    assert_eq!(cpu.d[0] & 0xff, 0x98);
    assert_eq!(cpu.sr.0 & 0x15, 0);
    let decoded = mdcore::cpu::disasm::disassemble(&[0x81, 0x01], 0x200);
    assert_eq!(decoded.text, "sbcd.b d1,d0");
    assert_eq!(decoded.length, 2);
}

#[test]
fn all_valid_decimal_pairs_match_arithmetic_with_extend_and_sticky_zero() {
    let bcd = |n: i32| (((n / 10) << 4) | (n % 10)) as u8;
    for add in [false, true] {
        for source in 0..100 {
            for destination in 0..100 {
                for extend in 0..2 {
                    let value: i32 = if add {
                        destination + source + extend
                    } else {
                        destination - source - extend
                    };
                    let borrow_or_carry = !(0..100).contains(&value);
                    let expected = bcd(value.rem_euclid(100));
                    for old_zero in [false, true] {
                        let cpu = run(
                            if add { 0xc101 } else { 0x8101 },
                            bcd(source),
                            bcd(destination),
                            if extend == 1 { 0x10 } else { 0 } | if old_zero { 4 } else { 0 },
                        );
                        assert_eq!(
                            cpu.d[0] & 0xff,
                            u32::from(expected),
                            "add={add} {destination} {source} X={extend}"
                        );
                        assert_eq!(cpu.sr.0 & 0x11, if borrow_or_carry { 0x11 } else { 0 });
                        assert_eq!(cpu.sr.0 & 4, if old_zero && expected == 0 { 4 } else { 0 });
                    }
                }
            }
        }
    }
}

#[test]
fn memory_bcd_predecrements_source_then_destination_with_a7_stride() {
    for add in [false, true] {
        for source in 0..8 {
            for destination in 0..8 {
                let mut bus = SparseBus::new();
                let opcode = if add { 0xc108 } else { 0x8108 } | (destination << 9) | source;
                bus.write16(0x200, opcode);
                let mut cpu = M68000::new(0x200);
                cpu.sr.0 = 0x2704;
                for (i, a) in cpu.a.iter_mut().enumerate() {
                    *a = 0x1000 + i as u32 * 0x100;
                }
                let before = cpu.a;
                let s = source as usize;
                let d = destination as usize;
                let source_address = before[s] - if s == 7 { 2 } else { 1 };
                let destination_address = before[d]
                    - if d == 7 { 2 } else { 1 }
                    - if s == d {
                        if s == 7 {
                            2
                        } else {
                            1
                        }
                    } else {
                        0
                    };
                bus.write8(source_address, 1);
                bus.write8(destination_address, 0x20);
                cpu.prime(&mut bus);
                let result = cpu.step(&mut bus);
                assert_eq!(result.exception, None);
                assert_eq!(result.cycles, 18);
                assert_eq!(
                    bus.read8(destination_address),
                    if add { 0x21 } else { 0x19 }
                );
                assert_eq!(bus.read8(source_address), 1);
                assert_eq!(cpu.a[d], destination_address);
                if s != d {
                    assert_eq!(cpu.a[s], source_address);
                }
                let decoded = mdcore::cpu::disasm::disassemble(&opcode.to_be_bytes(), 0x200);
                assert_eq!(
                    decoded.text,
                    format!(
                        "{}.b -(a{source}),-(a{destination})",
                        if add { "abcd" } else { "sbcd" }
                    )
                );
            }
        }
    }
}

#[test]
fn decimal_negate_uses_memory_access_cost_and_stack_byte_stride() {
    let mut bus = SparseBus::new();
    bus.write16(0x200, 0x4827);
    bus.write8(0x1000, 1);
    let mut cpu = M68000::new(0x200);
    cpu.a[7] = 0x1002;
    cpu.sr.0 = 0x2704;
    cpu.prime(&mut bus);
    let result = cpu.step(&mut bus);
    assert_eq!(result.exception, None);
    assert_eq!(result.cycles, 14);
    assert_eq!(cpu.a[7], 0x1000);
    assert_eq!(bus.read8(0x1000), 0x99);
    assert_eq!(cpu.sr.0 & 0x1f, 0x19);
}
