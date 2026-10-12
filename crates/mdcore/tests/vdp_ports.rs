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
    use mdcore::vdp::{Vdp, DOTS_PER_LINE};
    let mut v = Vdp::new();
    v.control_write(1, 0x20);
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
        v.vint_requests >= v.frame && v.vint_requests <= v.frame + 1,
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
    assert_eq!(&sys.vdp.vram[..204], &sys.cart.data[0x100..0x1CC]);
    sys.cpu.pc = 0xFF8000;
    sys.cpu.prefetch = [0x4E71, 0x4E71]; // NOP
    let (cycles, exception) = sys.step_instruction();
    assert_eq!(exception, None);
    assert_eq!(cycles, 4 + 3420u32.div_ceil(7));
    assert_eq!(sys.cpu.pc, 0xFF8002);
    assert_eq!(sys.vdp.dma_bus_stall_master_cycles, 3420);
    assert_eq!(sys.vdp.line, 1);
    sys.reset();
    assert_eq!(sys.vdp.dma_bus_stall_master_cycles, 0);
}
