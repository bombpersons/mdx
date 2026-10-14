use mdcore::{Bus, SparseBus, M68000};

#[test]
fn division_uses_operand_dependent_reference_body_and_fixed_ea_costs() {
    // Numeric expectations: independent GPGX UseDivuCycles/UseDivsCycles,
    // with m68ki_cycles[80C0/81C0]=0 and immediate entries=4 CPU clocks.
    for (opcode, dividend, divisor, cycles) in [
        (0x82c0, 0u32, 1u16, 136u32),
        (0x83c0, 0, 1, 150),
        (0x83c0, 1, 1, 150),
        (0x83c0, u32::MAX, 1, 156),
        (0x83c0, u32::MAX, u16::MAX, 154),
        (0x82c0, 0x10000, 1, 10),
        (0x83c0, 0x7fff0000, 1, 16),
        (0x83c0, 0x80000000, 0xffff, 154),
        (0x83c0, 0x80000000, 0x8000, 154),
    ] {
        let mut bus = SparseBus::new();
        bus.write16(0x200, opcode);
        let mut cpu = M68000::new(0x200);
        cpu.d[0] = u32::from(divisor);
        cpu.d[1] = dividend;
        cpu.prime(&mut bus);
        let outcome = cpu.step(&mut bus);
        assert_eq!(outcome.exception, None);
        assert_eq!(
            outcome.cycles, cycles,
            "opcode={opcode:04x} dividend={dividend:08x} divisor={divisor:04x}"
        );
    }
}
