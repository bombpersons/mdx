//! Numeric expectations are from GPGX's hardware-derived reset formula and
//! independently specified FIFO/68000 tables, not captured from mdcore.
use mdcore::{vdp::Vdp, Bus, Cartridge, SparseBus, System, M68000};

#[test]
fn absolute_short_addresses_are_sign_extended_in_decode_and_execution() {
    let bytes = [0x20, 0x38, 0xe0, 0x1c];
    let instruction = mdcore::cpu::disasm::disassemble(&bytes, 0x200);
    assert_eq!(instruction.text, "move.l 0xFFFFE01C.w,d0");
    let mut bus = SparseBus::new();
    bus.write32(0x200, u32::from_be_bytes(bytes));
    bus.write32(0x0000e01c, 0x11223344);
    bus.write32(0xffffe01c, 0xaabbccdd);
    let mut cpu = M68000::new(0x200);
    cpu.prime(&mut bus);
    assert_eq!(cpu.step(&mut bus).exception, None);
    assert_eq!(cpu.d[0], 0xaabbccdd);
    assert_eq!(cpu.pc, 0x204);
}

#[test]
fn static_absolute_long_bit_test_costs_twenty_cycles() {
    let mut bus = SparseBus::new();
    bus.write16(0x200, 0x0839);
    bus.write16(0x202, 6);
    bus.write32(0x204, 0xfff529);
    bus.write8(0xfff529, 0x40);
    let mut cpu = M68000::new(0x200);
    cpu.prime(&mut bus);
    let result = cpu.step(&mut bus);
    assert_eq!(result.exception, None);
    assert_eq!(result.cycles, 20);
    assert_eq!(cpu.pc, 0x208);
    assert_eq!(cpu.sr.0 & 4, 0);
}

#[test]
fn fifo_vram_needs_two_slots_and_full_fifo_stalls_in_both_widths() {
    for (wide, first, full_wait, drained) in [(false, 510, 511, 1770), (true, 820, 826, 1844)] {
        let mut v = Vdp::new();
        v.control_write(1, 0x44);
        v.control_write(12, u8::from(wide));
        v.code = 1;
        assert_eq!(v.fifo_status(), 0x200);
        for _ in 0..4 {
            assert_eq!(v.fifo_write_wait(0), 0);
        }
        assert_eq!(v.fifo_status(), 0x100);
        v.advance_master_cycles(first);
        assert_eq!(v.fifo_status(), 0);
        v.advance_master_cycles(drained - first);
        assert_eq!(v.fifo_status(), 0x200);
        let mut full = Vdp::new();
        full.control_write(1, 0x44);
        full.control_write(12, u8::from(wide));
        full.code = 1;
        for _ in 0..4 {
            full.fifo_write_wait(0);
        }
        assert_eq!(full.fifo_write_wait(0), full_wait);
        assert_eq!(full.fifo_stall_master_cycles, u64::from(full_wait));
    }
}

#[test]
fn cram_uses_one_slot_and_forced_blank_has_no_fifo_stall() {
    let mut v = Vdp::new();
    v.control_write(1, 0x44);
    v.code = 3;
    for _ in 0..4 {
        v.fifo_write_wait(0);
    }
    assert_eq!(v.fifo_write_wait(0), 231);
    v.control_write(1, 4);
    for _ in 0..20 {
        assert_eq!(v.fifo_write_wait(0), 0);
    }
}

#[test]
fn vint_delay_is_master_precise_and_chunk_independent() {
    for (wide, delay) in [(false, 770), (true, 788)] {
        let mut v = Vdp::new();
        v.control_write(1, 0x24);
        v.control_write(12, u8::from(wide));
        v.advance_master_cycles(224 * 3420);
        assert_eq!(v.frame, 1);
        assert!(!v.vint_pending);
        for _ in 0..delay - 1 {
            v.advance_master_cycles(1);
        }
        assert_eq!(v.vint_requests, 0);
        v.advance_master_cycles(1);
        assert!(v.vint_pending);
        assert_eq!(v.vint_requests, 1);
    }
}

#[test]
fn refresh_is_periodic_console_wait_not_a_nop_opcode_cost() {
    let mut bytes = vec![0; 0x1000];
    bytes[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    for word in bytes[0x200..].chunks_exact_mut(2) {
        word.copy_from_slice(&0x4e71u16.to_be_bytes());
    }
    let mut s = System::new(Cartridge::from_bytes("refresh.md", bytes).unwrap());
    // First dispatch is master784462; next refresh deadline is784896.
    // Sixteen NOPs put dispatch16 at784910, which must incur two extra clocks.
    for _ in 0..16 {
        assert_eq!(s.step_instruction(), (4, None));
    }
    assert_eq!(s.step_instruction(), (6, None));
    assert_eq!(s.step_instruction(), (4, None));
}
