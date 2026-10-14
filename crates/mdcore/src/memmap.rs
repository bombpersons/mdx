//! Mega Drive memory map.
//!
//! The 68000 drives A1..A23. Which device answers is decided by the high
//! address bits, not by absolute ranges, so an address the CPU drove with
//! unusual high bits can still land somewhere entirely different from where the
//! same low 24 bits would normally go. Decoding this correctly is what makes
//! the reset-vector behaviour of some carts explicable.

/// Cartridge aperture selected when A23 and A22 are both clear, giving the
/// 0x00000000-0x3FFFFFFF window that a ROM is mirrored through.
pub const CART_APERTURE_MASK: u32 = 0x00C0_0000;

/// Fold a full 32-bit address into the 4 MiB cartridge aperture.
pub fn fold_to_cart(addr: u32) -> u32 {
    addr & 0x003F_FFFF
}

/// Fold a full 32-bit address into the cartridge window, mirrored to the size of
/// the actual ROM. A 512 KiB cart, for instance, repeats every 0x80000 bytes.
pub fn fold_to_cart_mirrored(addr: u32, rom_len: usize) -> u32 {
    let a = fold_to_cart(addr);
    if rom_len == 0 {
        return a;
    }
    // Cartridges are mirrored at a power-of-two period, minimum 64 KiB
    // (`Cartridge::mirror_period` also applies the Sonic & Knuckles rule).
    let period = (rom_len.max(0x1_0000) as u32).next_power_of_two();
    a % period
}

/// Fold a full 32-bit address into the 64 KiB work-RAM aperture.
pub fn fold_to_wram(addr: u32) -> u32 {
    (addr & 0x003F_FFFF) & 0xFFFF
}

/// A decoded memory region, used by the tracer and by `mdx vdp map`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Cartridge,
    /// Z80 work RAM and its mirrors in the 68000's A0 aperture.
    Z80Ram,
    /// FM ports mirrored through A04000-A05FFF and A0C000-A0DFFF.
    Fm,
    Io,
    /// BUSREQ/RESET pages and the serial Z80 bank register.
    Z80Ctrl,
    Sram,
    Vdp,
    Wram,
    Unmapped,
}

impl Region {
    pub fn as_str(self) -> &'static str {
        match self {
            Region::Cartridge => "cartridge",
            Region::Z80Ram => "z80_ram",
            Region::Fm => "fm",
            Region::Io => "io",
            Region::Z80Ctrl => "z80_ctrl",
            Region::Sram => "sram",
            Region::Vdp => "vdp",
            Region::Wram => "wram",
            Region::Unmapped => "unmapped",
        }
    }
}

/// Classify the device that answers `addr`.
///
/// Decode, in order:
///   * A23 and A22 clear          -> cartridge
///   * A00000-A0FFFF              -> Z80 window
///   * A10000-A1001F              -> I/O
///   * A11100-A112FF              -> Z80 BUSREQ/RESET (mirrored byte lanes)
///   * A130F0-A130FF              -> cartridge save RAM
///   * C00000-C0001F              -> VDP
///   * E00000-FFFFFF              -> work RAM (64 KiB mirrored)
///   * anything else              -> unmapped and open bus on real hardware
pub fn region(addr: u32) -> Region {
    if addr & 0x00C0_0000 == 0 {
        return Region::Cartridge;
    }
    match addr & 0x00FF_FFFF {
        0x00A0_0000..=0x00A0_3FFF | 0x00A0_8000..=0x00A0_BFFF => Region::Z80Ram,
        0x00A0_4000..=0x00A0_5FFF | 0x00A0_C000..=0x00A0_DFFF => Region::Fm,
        0x00A0_6000..=0x00A0_60FF | 0x00A0_E000..=0x00A0_E0FF => Region::Z80Ctrl,
        0x00A1_0000..=0x00A1_001F => Region::Io,
        0x00A1_1100..=0x00A1_12FF => Region::Z80Ctrl,
        // $A130xx (!TIME): SRAM switch, bank registers and board-specific
        // registers, all through the cartridge's !TIME handlers.
        0x00A1_3000..=0x00A1_30FF => Region::Sram,
        0x00C0_0000..=0x00C0_001F => Region::Vdp,
        _ => {
            if addr & 0x00E0_0000 == 0x00E0_0000 {
                Region::Wram
            } else {
                Region::Unmapped
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_vector_decodes_by_a23_not_by_magnitude() {
        // King's Bounty. A23 is set, so this is work RAM, not cartridge, even
        // though 0xFFFF9C1E looks like a high address into the cart.
        assert_eq!(region(0xFFFF_9C1E), Region::Wram);
        assert_eq!(fold_to_wram(0xFFFF_9C1E), 0x9C1E);
    }

    #[test]
    fn clearing_the_aperture_bits_moves_the_same_vector_into_the_cartridge() {
        assert_eq!(region(0xFFFF_9C1E & !CART_APERTURE_MASK), Region::Cartridge);
        // Folded into the 4 MiB aperture, then mirrored across a 512 KiB cart.
        assert_eq!(fold_to_cart(0xFFFF_9C1E), 0x003F_9C1E);
        assert_eq!(fold_to_cart_mirrored(0xFFFF_9C1E, 0x80000), 0x0007_9C1E);
    }

    #[test]
    fn cartridge_aperture_stops_at_3fffff() {
        assert_eq!(region(0x0000_0000), Region::Cartridge);
        assert_eq!(region(0x0020_0000), Region::Cartridge);
        assert_eq!(region(0x003F_FFFF), Region::Cartridge);
        // Setting A22 leaves the aperture: 400000-7FFFFF is not decoded.
        assert_eq!(region(0x0040_0000), Region::Unmapped);
        assert_eq!(region(0x007F_FFFF), Region::Unmapped);
    }

    #[test]
    fn io_and_vdp_windows() {
        assert_eq!(region(0x00A1_0000), Region::Io);
        assert_eq!(region(0x00A1_000F), Region::Io);
        assert_eq!(region(0x00A1_1100), Region::Z80Ctrl);
        assert_eq!(region(0x00A1_30F1), Region::Sram);
        assert_eq!(region(0x00A0_0000), Region::Z80Ram);
        assert_eq!(region(0x00A0_4000), Region::Fm);
        assert_eq!(region(0x00C0_0000), Region::Vdp);
        assert_eq!(region(0x00C0_0004), Region::Vdp);
        // Past the VDP's 32-byte port window the address is simply not mapped.
        assert_eq!(region(0x00C0_0020), Region::Unmapped);
        assert_eq!(region(0x00FF_0000), Region::Wram);
    }

    #[test]
    fn sound_windows_decode_mirrors_and_both_control_byte_lanes() {
        for address in [0xa04003, 0xa04004, 0xa05fff, 0xa0c000, 0xa0dfff] {
            assert_eq!(region(address), Region::Fm);
        }
        assert_eq!(region(0xa0bfff), Region::Z80Ram);
        assert_eq!(region(0xa060ff), Region::Z80Ctrl);
        assert_eq!(region(0xa0e000), Region::Z80Ctrl);
        for address in [0xa11100, 0xa11101, 0xa11102, 0xa111ff, 0xa11200, 0xa112ff] {
            assert_eq!(region(address), Region::Z80Ctrl);
        }
    }

    #[test]
    fn addresses_between_apertures_are_unmapped() {
        assert_eq!(region(0x0040_0000), Region::Unmapped);
        assert_eq!(region(0x0080_0000), Region::Unmapped);
    }
}
