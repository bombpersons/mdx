use mdcore::{vdp::Vdp, Bus, Cartridge, System};

#[test]
fn cram_unused_bits_come_from_next_fifo_slot_not_last_data_word() {
    let mut v = Vdp::new();
    v.registers[15] = 2;
    v.code = 1;
    // Independent EWJ frame497 capture: FIFO [1,1,0,0], next-write index1.
    for word in [1, 1, 0, 0, 1] {
        v.data_write(word);
    }
    v.code = 8;
    v.address_latch = 0;
    assert_eq!(v.data_read(), 1);
    assert_eq!(v.data_read(), 1);
    // Independent frame4419 capture: last word would supply0x0011,
    // but next slot supplies0x0111 even with display disabled/FIFO empty.
    let mut v = Vdp::new();
    v.registers[15] = 2;
    v.code = 1;
    for word in [0x0133, 0x0a12, 0xa484, 0x00f7] {
        v.data_write(word);
    }
    v.code = 8;
    v.address_latch = 0;
    for _ in 0..64 {
        assert_eq!(v.data_read(), 0x0111);
    }
    assert!(v.cram.iter().all(|byte| *byte == 0));
    v.code = 3;
    v.address_latch = 0;
    v.data_write(0x0111);
    assert_eq!(&v.cram[..2], &[0, 0]);
}

#[test]
fn vsram_alias_and_byte_vram_read_preserve_undriven_fifo_lanes() {
    let mut v = Vdp::new();
    v.code = 1;
    for word in [0xf800, 0, 0, 0] {
        v.data_write(word);
    }
    v.vsram[..2].copy_from_slice(&[7, 0xff]);
    v.vram[..2].copy_from_slice(&[0xab, 0xcd]);
    v.code = 4;
    v.address_latch = 0x50;
    assert_eq!(v.data_read(), 0xffff);
    v.code = 12;
    v.address_latch = 0;
    assert_eq!(v.data_read(), 0xf8cd);
    v.code = 0;
    v.address_latch = 0;
    assert_eq!(v.data_read(), 0xabcd);
}

#[test]
fn hcounter_matches_all_independent_standard_ntsc_master_clocks() {
    let reference = include_bytes!("fixtures/ntsc_hcounter.bin");
    for mode in 0..2 {
        let mut v = Vdp::new();
        v.control_write(1, 4);
        v.control_write(12, mode as u8);
        for master in 0..3420 {
            assert_eq!(
                v.hv_counter() as u8,
                reference[mode * 3420 + master],
                "mode={mode} master={master}"
            );
            v.advance_master_cycles(1);
        }
    }
}

#[test]
fn vertical_counter_rollover_and_mode5_latch_follow_hardware() {
    let mut v = Vdp::new();
    v.control_write(1, 4);
    for (line, expected) in [(234, 234), (235, 229), (261, 255)] {
        v.line = line;
        assert_eq!(v.hv_counter() >> 8, expected);
    }
    v.line = 100;
    let value = v.hv_counter();
    v.control_write(0, 2);
    v.advance_master_cycles(3440);
    assert_eq!(v.hv_counter(), value);
    v.control_write(0, 0);
    assert_ne!(v.hv_counter(), value);
    v.control_write(1, 0);
    v.line = 219;
    assert_eq!(v.hv_counter() >> 8, 213);
}

#[test]
fn system_hv_byte_lanes_and_port_aliases_do_not_consume_vdp_data() {
    let mut bytes = vec![0; 1024];
    bytes[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    let mut s = System::new(Cartridge::from_bytes("counter.md", bytes).unwrap());
    s.vdp.control_pending = true;
    s.vdp.address_latch = 0x20;
    let value = s.vdp.hv_counter();
    for port in [0xc00008, 0xc0000a, 0xc0000c, 0xc0000e] {
        assert_eq!(s.read16(port), value);
        assert_eq!(s.read8(port), (value >> 8) as u8);
        assert_eq!(s.read8(port + 1), value as u8);
    }
    assert!(s.vdp.control_pending);
    assert_eq!(s.vdp.address_latch, 0x20);
}
