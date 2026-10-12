//! Operand sizes and effective-address computation.

use crate::cpu::bus::Bus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Byte,
    Word,
    Long,
}

impl Size {
    #[inline]
    pub fn bytes(self) -> u32 {
        match self {
            Size::Byte => 1,
            Size::Word => 2,
            Size::Long => 4,
        }
    }

    /// Number of low bits of a value that participate in the operation.
    #[inline]
    pub fn mask(self) -> u32 {
        match self {
            Size::Byte => 0xFF,
            Size::Word => 0xFFFF,
            Size::Long => 0xFFFF_FFFF,
        }
    }

    #[inline]
    pub fn from_bits(bits: u16) -> Size {
        match bits & 3 {
            0 => Size::Byte,
            1 => Size::Word,
            _ => Size::Long,
        }
    }
}

/// A decoded effective address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ea {
    /// Data register direct; the operand is in `reg`.
    Data(u8),
    /// Address register direct; the operand is the address itself.
    Address(u8),
    /// A resolved memory address.
    Memory(u32),
    /// An immediate operand, already read from the instruction stream.
    Immediate(u32),
}

impl Ea {
    pub fn is_direct(&self) -> bool {
        matches!(self, Ea::Data(_) | Ea::Address(_))
    }

    pub fn is_memory(&self) -> bool {
        matches!(self, Ea::Memory(_))
    }
}

/// The addressing mode bits of an instruction word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    DataRegisterDirect,
    AddressRegisterDirect,
    AddressRegisterIndirect,
    AddressRegisterIndirectPostincrement,
    AddressRegisterIndirectPredecrement,
    AddressRegisterIndirectDisplacement,
    AddressRegisterIndirectIndex,
    AbsoluteShort,
    AbsoluteLong,
    Immediate,
    ProgramCounterRelative,
    ProgramCounterRelativeIndex,
}

/// Decode an instruction word's mode/reg pair.
pub fn decode_mode(mode: u16, reg: u8) -> Mode {
    match mode {
        0 => Mode::DataRegisterDirect,
        1 => Mode::AddressRegisterDirect,
        2 => Mode::AddressRegisterIndirect,
        3 => Mode::AddressRegisterIndirectPostincrement,
        4 => Mode::AddressRegisterIndirectPredecrement,
        5 => Mode::AddressRegisterIndirectDisplacement,
        6 => Mode::AddressRegisterIndirectIndex,
        7 => match reg {
            0 => Mode::AbsoluteShort,
            1 => Mode::AbsoluteLong,
            2 => Mode::ProgramCounterRelative,
            3 => Mode::ProgramCounterRelativeIndex,
            4 => Mode::Immediate,
            _ => Mode::AbsoluteShort,
        },
        _ => Mode::AbsoluteShort,
    }
}

impl crate::cpu::m68000::M68000 {
    /// Apply a brief-address extension word: an 8-bit sign-extended offset plus
    /// an index register and scale.
    fn brief_index<B: Bus>(&mut self, bus: &mut B, base: u32) -> u32 {
        let ext = self.next_word(bus);
        // Bits 15-12 select the index register: 0-7 are data, 8-15 address.
        let selector = ((ext >> 12) & 0xF) as usize;
        let long = ext & 0x0800 != 0;
        let register = if selector < 8 {
            self.d[selector]
        } else {
            self.a[selector - 8]
        };
        let index_raw = if long {
            register
        } else {
            register as u16 as i16 as i32 as u32
        };
        let disp = (ext as u8 as i8) as i32 as u32;
        base.wrapping_add(index_raw).wrapping_add(disp)
    }

    /// Resolve an effective address.
    ///
    /// `size` only matters for the increment/decrement modes, which adjust by
    /// 1 for byte operations and by 2 otherwise.
    pub fn effective_address<B: Bus>(
        &mut self,
        bus: &mut B,
        mode: Mode,
        reg: u8,
        size: Size,
    ) -> Ea {
        match mode {
            Mode::DataRegisterDirect => Ea::Data(reg),
            Mode::AddressRegisterDirect => Ea::Address(reg),
            Mode::AddressRegisterIndirect => Ea::Memory(self.a[reg as usize]),
            Mode::AddressRegisterIndirectPostincrement => {
                let addr = self.a[reg as usize];
                let step = if size == Size::Byte && reg == 7 {
                    2
                } else {
                    size.bytes()
                };
                self.a[reg as usize] = self.a[reg as usize].wrapping_add(step);
                Ea::Memory(addr)
            }
            Mode::AddressRegisterIndirectPredecrement => {
                let step = if size == Size::Byte && reg == 7 {
                    2
                } else {
                    size.bytes()
                };
                self.a[reg as usize] = self.a[reg as usize].wrapping_sub(step);
                Ea::Memory(self.a[reg as usize])
            }
            Mode::AddressRegisterIndirectDisplacement => {
                let base = self.a[reg as usize];
                let disp = self.next_word(bus) as i16 as i32;
                Ea::Memory(base.wrapping_add(disp as u32))
            }
            Mode::AddressRegisterIndirectIndex => {
                let base = self.a[reg as usize];
                Ea::Memory(self.brief_index(bus, base))
            }
            Mode::AbsoluteShort => {
                let addr = self.next_word(bus) as i16 as i32 as u32;
                Ea::Memory(addr)
            }
            Mode::AbsoluteLong => {
                let hi = self.next_word(bus) as u32;
                let lo = self.next_word(bus) as u32;
                Ea::Memory(hi << 16 | lo)
            }
            Mode::Immediate => {
                let value = self.read_immediate(bus, size);
                Ea::Immediate(value)
            }
            Mode::ProgramCounterRelative => {
                let disp = self.next_word(bus) as i16 as i32;
                Ea::Memory(self.pc.wrapping_sub(2).wrapping_add(disp as u32))
            }
            Mode::ProgramCounterRelativeIndex => {
                // The base is the address of the extension word itself.
                let base = self.pc;
                Ea::Memory(self.brief_index(bus, base))
            }
        }
    }

    /// Read an immediate operand of the given size from the instruction stream.
    pub(crate) fn read_immediate<B: Bus>(&mut self, bus: &mut B, size: Size) -> u32 {
        match size {
            Size::Byte => (self.next_word(bus) & 0xFF) as u32,
            Size::Word => self.next_word(bus) as u32,
            Size::Long => {
                let hi = self.next_word(bus) as u32;
                let lo = self.next_word(bus) as u32;
                hi << 16 | lo
            }
        }
    }

    /// Resolve a control effective address for LEA/PEA without reading memory.
    /// Mode 111 uses the ordinary absolute-word, absolute-long, PC-displacement,
    /// PC-index table. PC-relative bases are the extension word's own address.
    pub fn lea_address<B: Bus>(&mut self, bus: &mut B, mode: u16, reg: u8) -> u32 {
        if mode != 7 {
            // The ordinary effective-address modes. LEA computes the address
            // without reading it, so all of these just yield An - but the
            // displacement forms still have to be stepped over.
            match mode {
                5 => {
                    let disp = self.next_word(bus) as i16 as i32;
                    self.a[reg as usize].wrapping_add(disp as u32)
                }
                6 => self.brief_index(bus, self.a[reg as usize]),
                _ => self.a[reg as usize],
            }
        } else {
            match reg {
                0 => self.next_word(bus) as i16 as i32 as u32,
                2 => {
                    let base = self.pc;
                    let disp = self.next_word(bus) as i16 as i32;
                    base.wrapping_add(disp as u32)
                }
                3 => {
                    let base = self.pc;
                    self.brief_index(bus, base)
                }
                _ => {
                    let hi = self.next_word(bus) as u32;
                    let lo = self.next_word(bus) as u32;
                    hi << 16 | lo
                }
            }
        }
    }

    /// Program-counter-relative displacement, as used by branches.
    /// A 16-bit branch displacement.
    ///
    /// The base is the address of the displacement word itself, which is where
    /// the internal program counter sits while the displacement is being added.
    pub(crate) fn pc_displacement<B: Bus>(&mut self, bus: &mut B) -> u32 {
        // The base is the displacement word's OWN address. next_word has
        // already advanced past it, so this is two bytes back - Genesis Plus
        // GX writes the same thing out as REG_PC -= 2 before branching.
        let disp = self.next_word(bus) as i16 as i32;
        self.pc.wrapping_sub(2).wrapping_add(disp as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_masks_and_widths() {
        assert_eq!(Size::Byte.bytes(), 1);
        assert_eq!(Size::Word.bytes(), 2);
        assert_eq!(Size::Long.bytes(), 4);
        assert_eq!(Size::from_bits(0), Size::Byte);
        assert_eq!(Size::from_bits(1), Size::Word);
        assert_eq!(Size::from_bits(2), Size::Long);
        assert_eq!(Size::from_bits(3), Size::Long);
    }

    #[test]
    fn mode_7_variants() {
        assert_eq!(decode_mode(7, 0), Mode::AbsoluteShort);
        assert_eq!(decode_mode(7, 1), Mode::AbsoluteLong);
        assert_eq!(decode_mode(7, 2), Mode::ProgramCounterRelative);
        assert_eq!(decode_mode(7, 3), Mode::ProgramCounterRelativeIndex);
        assert_eq!(decode_mode(7, 4), Mode::Immediate);
    }
}
