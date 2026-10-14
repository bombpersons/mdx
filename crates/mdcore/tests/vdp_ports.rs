//! Port-level VDP tests.
//!
//! The cartridge's own initialisation is the reference here: `move.w #$8004,
//! (a4)` with a4 = $C00004 is how every Genesis game sets VDP register 0, and
//! the boot routine at 0x27DEC writes sixteen registers that way. If the
//! control port decode is wrong the machine still runs, still clears VRAM, and
//! still reports no exceptions - it simply never turns the display on.

use mdcore::cartridge::Cartridge;
use mdcore::system::System;
use mdcore::Bus;

fn cartridge() -> Cartridge {
    let mut bytes = vec![0u8; 512 * 1024];
    bytes[..4].copy_from_slice(&0xFFFF1000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    // Authored, nonzero DMA source catches accidental zero-filled transfers.
    for (i, value) in bytes[0x100..0x1cc].iter_mut().enumerate() {
        *value = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    Cartridge::from_bytes("synthetic-vdp.md", bytes).unwrap()
}

#[test]
fn reset_and_completed_frames_use_the_visible_end_vblank_edge() {
    use mdcore::vdp::MASTER_CYCLES_PER_LINE;
    let mut sys = System::new(cartridge());
    assert_eq!(
        sys.vdp.line, 159,
        "hardware-measured NTSC CPU release phase"
    );
    assert_eq!(sys.vdp.dot, 335);
    assert_eq!(sys.vdp.frame, 0);
    assert_eq!(sys.vdp.vint_requests, 0, "reset is not an interrupt edge");
    sys.vdp.control_write(1, 4);
    // First frame remains 192 lines despite enabling Mode5 after release.
    sys.vdp
        .advance_master_cycles(33 * MASTER_CYCLES_PER_LINE - 1282);
    assert_eq!(sys.vdp.line, 224);
    assert_eq!(sys.vdp.frame, 1);
    assert_eq!(
        sys.vdp.vint_requests, 0,
        "VINT follows, not coincides with, VBLANK"
    );
    sys.vdp.advance_master_cycles(769);
    assert_eq!(sys.vdp.vint_requests, 0);
    sys.vdp.advance_master_cycles(1);
    assert_eq!(sys.vdp.vint_requests, 1);
    sys.vdp.advance_master_cycles(19 * MASTER_CYCLES_PER_LINE);
    assert!(sys.vdp.vblank, "NTSC line 243 remains inside VBLANK");
    sys.vdp.advance_master_cycles(18 * MASTER_CYCLES_PER_LINE);
    assert_eq!(sys.vdp.line, 261);
    assert!(!sys.vdp.vblank);
}

#[test]
fn internal_copy_is_busy_for_its_reference_slots_without_cpu_bus_stalls() {
    let mut sys = System::new(cartridge());
    for (reg, value) in [
        (1, 0x14u16),
        (12, 1),
        (15, 1),
        (19, 3),
        (20, 0),
        (21, 4),
        (22, 0),
        (23, 0xc0),
    ] {
        sys.write16(0xc00004, 0x8000 | reg << 8 | value);
    }
    sys.vdp.vram[5] = 0xab;
    sys.vdp.vram[4] = 0xcd;
    sys.vdp.vram[7] = 0xef;
    sys.write32(0xc00004, 0x000000c0);
    assert_eq!(
        [sys.vdp.vram[1], sys.vdp.vram[0], sys.vdp.vram[3]],
        [0xab, 0xcd, 0xef]
    );
    assert_eq!(sys.vdp.registers[21], 7);
    assert_eq!(sys.vdp.dma_bus_stall_master_cycles, 0);
    sys.vdp.advance_master_cycles(99);
    assert_eq!(sys.read16(0xc00004) & 2, 2);
    sys.vdp.advance_master_cycles(1);
    assert_eq!(sys.read16(0xc00004) & 2, 0);
}

#[test]
fn fill_busy_survives_the_data_write_and_expires_without_stalling_the_cpu() {
    let mut sys = System::new(cartridge());
    for (reg, value) in [
        (1, 0x14u16),
        (12, 1),
        (15, 1),
        (19, 204),
        (20, 0),
        (21, 3),
        (22, 0),
        (23, 0x80),
    ] {
        sys.write16(0xC00004, 0x8000 | reg << 8 | value);
    }
    sys.write32(0xC00004, 0x40000080);
    assert_eq!(
        sys.read16(0xC00004) & 2,
        2,
        "fill is busy awaiting its data word"
    );
    sys.write16(0xC00000, 0xABCD);
    assert!(!sys.vdp.pending_dm);
    assert_eq!(
        sys.read16(0xC00004) & 2,
        2,
        "writing data does not finish the timed operation"
    );
    assert_eq!(sys.vdp.registers[21], 207);
    sys.cpu.pc = 0xFF8000;
    sys.cpu.prefetch = [0x4e71, 0x4e71];
    let (cycles, error) = sys.step_instruction();
    assert_eq!(error, None);
    assert_eq!(cycles, 4);
    assert_eq!(sys.cpu.pc, 0xFF8002);
    assert_eq!(sys.vdp.dma_bus_stall_master_cycles, 0);
    sys.vdp.advance_master_cycles(3420);
    assert_eq!(sys.read16(0xC00004) & 2, 2);
    sys.vdp.advance_master_cycles(32);
    assert_eq!(sys.read16(0xC00004) & 2, 0);
    sys.reset();
    assert!(!sys.vdp.dma_busy());
}

/// The exact sequence the cartridge uses to configure the VDP.
#[test]
fn cartridge_register_writes_through_the_control_port() {
    let mut sys = System::new(cartridge());

    // movea.l #$c00004,a4 ; movea.l #$c00000,a5
    let control = 0x00C0_0004u32;
    let data = 0x00C0_0000u32;

    for (reg, value) in [
        (0, 0x04),
        (1, 0x34),
        (2, 0x28),
        (3, 0x30),
        (4, 0x05),
        (5, 0x6E),
        (7, 0x00),
        (0x0B, 0x03),
        (0x0C, 0x81),
        (0x0D, 0x08),
        (0x0F, 0x02),
        (0x10, 0x00),
        (0x11, 0x00),
    ] {
        // 10RRRRRR DDDDDDDD
        sys.write16(control, 0x8000 | ((reg as u16) << 8) | value as u16);
        assert_eq!(
            sys.vdp.registers[reg], value,
            "VDP register {reg} did not take the value written through $C000{:04X}",
            reg
        );
    }

    // These commands are independent hardware encodings, not the old
    // implementation's low-two-bit destination convention.
    sys.write32(control, 0x4000_0000);
    sys.write16(data, 0x1234);
    assert_eq!(&sys.vdp.vram[..2], &[0x12, 0x34]);
    assert!(sys.vdp.cram.iter().all(|c| *c == 0));
    sys.write32(control, 0xC000_0000);
    sys.write16(data, 0x0EEE);
    assert_eq!(&sys.vdp.cram[..2], &[0x0E, 0xEE]);
    sys.write32(control, 0x4000_0010);
    sys.write16(data, 0x0123);
    assert_eq!(&sys.vdp.vsram[..2], &[0x01, 0x23]);
    assert_eq!(sys.vdp_data_counts, [1, 1, 1]);
    // The second command word supplies address bits 15-14.
    sys.write32(control, 0x4000_0003);
    sys.write16(data, 0xABCD);
    assert_eq!(&sys.vdp.vram[0xC000..0xC002], &[0xAB, 0xCD]);
}

/// A vertical interrupt must be raised once per frame while the enable bit is
/// set. The dot counter is stepped in 68000-sized chunks so the blanking
/// boundary is genuinely crossed, rather than teleporting the line counter.
#[test]
fn vertical_interrupt_is_raised_once_per_frame_when_enabled() {
    use mdcore::vdp::Vdp;
    let mut v = Vdp::new();
    v.control_write(1, 0x24);
    // One frame is 262 lines of 896 dots; a 68000 instruction is about
    // 44 cycles of 7 dots, so 1200 of them is nearly three frames.
    for _ in 0..1200 {
        v.advance(44 * 7);
    }
    assert!(v.frame > 0, "no frame elapsed");
    // The frame counter is bumped when the line counter wraps, which happens
    // just *after* the blanking edge, so the request count can lead it by one
    // for the frame in progress.
    assert!(
        v.vint_requests <= v.frame && v.vint_requests + 1 >= v.frame,
        "one vertical interrupt per frame while enabled: {} requests over {} frames",
        v.vint_requests,
        v.frame
    );
}

#[test]
fn external_dma_advances_devices_while_the_cpu_waits_and_reset_clears_queued_waits() {
    let mut sys = System::new(cartridge());
    for (reg, value) in [
        (1, 0x14u16),
        (12, 1),
        (15, 2),
        (19, 102),
        (20, 0),
        (21, 0x80),
        (22, 0),
        (23, 0),
    ] {
        sys.write16(0xC00004, 0x8000 | reg << 8 | value);
    }
    sys.write32(0xC00004, 0x4000_0080);
    // The first update delivers 63 words at once; the rest arrive at the next
    // line start while the 68000 waits (reference per-update transfer).
    assert_eq!(&sys.vdp.vram[..126], &sys.cart.data[0x100..0x17E]);
    assert_eq!(sys.vdp.dma_queue.len(), 39);
    sys.cpu.pc = 0xFF8000;
    sys.cpu.prefetch = [0x4E71, 0x4E71]; // NOP
    let (cycles, exception) = sys.step_instruction();
    assert_eq!(exception, None);
    // Reset release is 1282 clocks into this scanline. Blank H40 DMA has
    // floor(2138*102/3420)=63 words left; the remaining39 need1307 clocks.
    let wait: u32 = (3420 - 1282) + 39 * 3420 / 102;
    assert_eq!(wait, 3445);
    assert_eq!(cycles, 4 + wait.div_ceil(7));
    assert_eq!(sys.cpu.pc, 0xFF8002);
    assert_eq!(sys.vdp.dma_bus_stall_master_cycles, u64::from(wait));
    assert_eq!(sys.vdp.line, 160);
    assert_eq!(&sys.vdp.vram[..204], &sys.cart.data[0x100..0x1CC]);
    assert!(sys.vdp.dma_queue.is_empty());
    sys.reset();
    assert_eq!(sys.vdp.dma_bus_stall_master_cycles, 0);
}
