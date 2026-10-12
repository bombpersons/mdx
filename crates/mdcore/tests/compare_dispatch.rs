//! Independent opcode-family boundaries: CMPA.L register operands overlap the
//! broad CMPM addressing mask but must never dereference or increment An.
use mdcore::{Bus, SparseBus, M68000};

#[test]
fn cmpa_long_address_registers_never_take_the_cmpm_path() {
    for src in 0..8 {
        for dst in 0..8 {
            let mut bus = SparseBus::new();
            bus.write16(0x200, 0xB1C8 | (dst << 9) | src);
            let mut cpu = M68000::new(0x200);
            for (index, address) in cpu.a.iter_mut().enumerate() {
                *address = 0x10000 + index as u32 * 0x1000;
            }
            cpu.sr.0 = 0x2710;
            let before = cpu.a;
            cpu.prime(&mut bus);
            let outcome = cpu.step(&mut bus);
            assert_eq!(outcome.exception, None, "src={src} dst={dst}");
            assert_eq!(cpu.a, before, "CMPA must preserve every address register");
            assert_eq!(cpu.pc, 0x202);
            assert_eq!(outcome.cycles, 6);
            let flags = 0x10
                | if src == dst {
                    4
                } else if src > dst {
                    9
                } else {
                    0
                };
            assert_eq!(cpu.sr.0 & 0x1f, flags, "src={src} dst={dst}");
        }
    }
}

#[test]
fn cmpm_all_valid_sizes_still_compare_memory_and_postincrement() {
    for (encoding, size, cycles) in [(0xB708, 1u32, 12), (0xB748, 2, 12), (0xB788, 4, 20)] {
        let mut bus = SparseBus::new();
        bus.write16(0x200, encoding);
        match size {
            1 => {
                bus.write8(0x1000, 3);
                bus.write8(0x2000, 9);
            }
            2 => {
                bus.write16(0x1000, 3);
                bus.write16(0x2000, 9);
            }
            _ => {
                bus.write32(0x1000, 3);
                bus.write32(0x2000, 9);
            }
        }
        let mut cpu = M68000::new(0x200);
        cpu.a[0] = 0x1000;
        cpu.a[3] = 0x2000;
        cpu.sr.0 = 0x2710;
        cpu.prime(&mut bus);
        let outcome = cpu.step(&mut bus);
        assert_eq!(outcome.exception, None);
        assert_eq!(outcome.cycles, cycles);
        assert_eq!(cpu.a[0], 0x1000 + size);
        assert_eq!(cpu.a[3], 0x2000 + size);
        assert_eq!(cpu.sr.0 & 0x1f, 0x10);
    }
}

#[test]
fn signed_divide_immediate_dispatches_for_every_destination_and_sign() {
    for target in 0..8 {
        for (dividend, divisor, expected, flags) in [
            (7i32, 4i16, 0x00030001u32, 0x10),
            (-7, 4, 0xfffdffff, 0x18),
            (7, -4, 0x0003ffff, 0x18),
            (-7, -4, 0xfffd0001, 0x10),
            (0, 4, 0, 0x14),
        ] {
            let mut bus = SparseBus::new();
            bus.write16(0x200, 0x81fc | target << 9);
            bus.write16(0x202, divisor as u16);
            let mut cpu = M68000::new(0x200);
            cpu.d[target as usize] = dividend as u32;
            cpu.sr.0 = 0x2710;
            cpu.prime(&mut bus);
            let outcome = cpu.step(&mut bus);
            assert_eq!(outcome.exception, None);
            assert_eq!(cpu.pc, 0x204);
            assert_eq!(cpu.d[target as usize], expected);
            assert_eq!(cpu.sr.0 & 0x1f, flags);
        }
    }
}

#[test]
fn signed_divide_host_overflow_cannot_panic_the_emulator() {
    let mut bus = SparseBus::new();
    bus.write16(0x200, 0x81fc);
    bus.write16(0x202, 0xffff);
    let mut cpu = M68000::new(0x200);
    cpu.d[0] = 0x80000000;
    cpu.prime(&mut bus);
    assert_eq!(cpu.step(&mut bus).exception, None);
    assert_eq!(cpu.d[0], 0);
    assert_eq!(cpu.sr.0 & 0x1f, 4);
}

#[test]
#[ignore = "reference timing probe: currently approximate; requires wider scheduling verification before integration"]
fn bit_operation_cycles_follow_register_memory_and_immediate_forms() {
    for (opcode, bit, expected_cycles, immediate, memory) in [
        (0x0300, 0, 6, false, false), // BTST D1,D0 (decompression hot path)
        (0x0340, 0, 6, false, false), // BCHG low word
        (0x0340, 16, 8, false, false),
        (0x0380, 0, 8, false, false), // BCLR low word
        (0x0380, 16, 10, false, false),
        (0x03c0, 0, 6, false, false),
        (0x0800, 0, 10, true, false),
        (0x0310, 0, 8, false, true), // BTST D1,(A0)
        (0x0350, 0, 12, false, true),
        (0x0810, 0, 12, true, true),
        (0x0850, 0, 16, true, true),
    ] {
        let mut bus = SparseBus::new();
        bus.write16(0x200, opcode);
        bus.write16(0x202, bit);
        let mut cpu = M68000::new(0x200);
        cpu.d[1] = bit as u32;
        cpu.d[0] = 1;
        cpu.a[0] = 0x1000;
        bus.write8(0x1000, 1);
        cpu.prime(&mut bus);
        let result = cpu.step(&mut bus);
        assert_eq!(result.exception, None);
        assert_eq!(result.cycles, expected_cycles, "{opcode:04x} bit {bit}");
        assert_eq!(cpu.pc, if immediate { 0x204 } else { 0x202 });
        if opcode & 0xc0 == 0 {
            assert_eq!(cpu.d[0], 1);
            if memory {
                assert_eq!(bus.read8(0x1000), 1);
            }
        }
    }
}
