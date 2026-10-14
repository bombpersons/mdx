//! The 68000 status register.
//!
//! Bit layout:
//!
//! ```text
//!  15   14 13 12 11 10  9  8  7  6  5  4  3  2  1  0
//!   T    -  S  -  - IIL IIL   -  -  -  X  N  Z  V  C
//! ```
//!
//! `X` is the carry/borrow bit shared by arithmetic and shifts; the other
//! condition codes are always written together.

pub const C: u16 = 0x0001;
pub const V: u16 = 0x0002;
pub const Z: u16 = 0x0004;
pub const N: u16 = 0x0008;
pub const X: u16 = 0x0010;
pub const CCR_MASK: u16 = C | V | Z | N | X;

/// The interrupt priority mask is bits 10-8.
///
/// The 68000 has **no separate interrupt-enable bit**: the mask does double
/// duty, and level 7 means "service nothing". That is the whole difference
/// from the 68010, which added a true I flag. Bit 13 is the supervisor bit
/// and bit 14 is unused.
///
/// This layout is not from memory. Genesis Plus GX's m68k core packs its SR as
/// `FLAG_T1 | (FLAG_S << 11) | FLAG_INT_MASK | ccr`, where `SFLAG_SET` is 4 -
/// so `FLAG_S << 11` lands on bit 13, and it sets `FLAG_INT_MASK = 0x0700` on
/// exception entry. Reinterpreting bit 13 as an interrupt-enable and bit 12-10
/// as the mask is a very plausible mistake that costs a day: it makes a CPU
/// whose mask is genuinely 7 - i.e. one correctly refusing interrupts - look
/// like a CPU stuck waiting to be interrupted.
pub const INTERRUPT_MASK: u16 = 0x0700;
pub const SUPERVISOR: u16 = 0x2000;
pub const TRACE: u16 = 0x8000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct StatusRegister(pub u16);

impl StatusRegister {
    #[inline]
    pub fn flag(&self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    #[inline]
    pub fn set_flag(&mut self, bit: u16, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    #[inline]
    pub fn ccr(&self) -> u16 {
        self.0 & CCR_MASK
    }

    #[inline]
    pub fn set_ccr(&mut self, value: u16) {
        self.0 = (self.0 & !CCR_MASK) | (value & CCR_MASK);
    }

    #[inline]
    pub fn interrupt_mask(&self) -> u8 {
        ((self.0 & INTERRUPT_MASK) >> 8) as u8
    }

    #[inline]
    pub fn set_interrupt_mask(&mut self, level: u8) {
        self.0 = (self.0 & !INTERRUPT_MASK) | ((level as u16) << 8 & INTERRUPT_MASK);
    }

    /// A 68000 CPU is "enabled" whenever its mask is below the level in
    /// question; there is no separate flag to consult. This exists so callers
    /// read as what they mean rather than repeating the arithmetic.
    #[inline]
    pub fn accepts(&self, level: u8) -> bool {
        self.interrupt_mask() < level
    }

    #[inline]
    pub fn is_supervisor(&self) -> bool {
        self.flag(SUPERVISOR)
    }

    #[inline]
    pub fn set_supervisor(&mut self, on: bool) {
        self.set_flag(SUPERVISOR, on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ccr_writes_preserve_the_rest_of_the_register() {
        let mut sr = StatusRegister(SUPERVISOR | 0x0700 | Z);
        sr.set_ccr(C | X);
        assert_eq!(sr.0, SUPERVISOR | 0x0700 | C | X);
        assert!(sr.is_supervisor());
    }

    #[test]
    fn interrupt_mask_round_trips() {
        let mut sr = StatusRegister::default();
        sr.set_interrupt_mask(6);
        assert_eq!(sr.interrupt_mask(), 6);
        assert_eq!(sr.0, 0x0600);
    }
}
