//! Memory bus abstraction.
//!
//! The CPU never talks to memory directly. This keeps three very different
//! backings behind one interface: the real Mega Drive map, a sparse map used by
//! the instruction test suite, and anything else an analysis pass wants to
//! interpose (a VRAM write tracer, for instance).

/// A memory bus seen from the 68000.
///
/// Addresses are full 32-bit values exactly as the CPU drove them, including
/// the high bits that select the device. The default word and long
/// implementations compose from bytes, which matches the 68000's behaviour for
/// RAM access.
pub trait Bus {
    /// The CPU's instruction-stream position moved (next unread word). Buses
    /// that model open-bus reads use it; others ignore it.
    fn note_pc(&mut self, _pc: u32) {}

    fn read8(&mut self, addr: u32) -> u8;
    fn write8(&mut self, addr: u32, value: u8);

    /// Big-endian: the most significant byte is stored at the lower address.
    fn read16(&mut self, addr: u32) -> u16 {
        let hi = self.read8(addr) as u16;
        let lo = self.read8(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    /// The bus is big-endian, so the word at `addr` is the high half of the
    /// long and the word at `addr + 2` is the low half.
    fn read32(&mut self, addr: u32) -> u32 {
        let hi = self.read16(addr) as u32;
        let lo = self.read16(addr.wrapping_add(2)) as u32;
        (hi << 16) | lo
    }

    /// Big-endian: the most significant byte is stored at the lower address.
    fn write16(&mut self, addr: u32, value: u16) {
        self.write8(addr, (value >> 8) as u8);
        self.write8(addr.wrapping_add(1), (value & 0xFF) as u8);
    }

    /// Big-endian, matching `read32`: the high half goes at the lower address.
    fn write32(&mut self, addr: u32, value: u32) {
        self.write16(addr, (value >> 16) as u16);
        self.write16(addr.wrapping_add(2), (value & 0xFFFF) as u16);
    }

    /// Instruction fetch. Split out because a tracer may want to see fetches
    /// without duplicating the read path.
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.read16(addr)
    }
}

/// A flat byte-addressed memory used by the instruction test suite.
///
/// Only the cells the suite declares are backed; everything else reads as zero
/// and discards writes. That matches the suite's own model, where the RAM list
/// enumerates the only locations whose contents are meaningful.
#[derive(Default)]
pub struct SparseBus {
    cells: std::collections::HashMap<u32, u8>,
}

impl SparseBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_byte(&mut self, addr: u32, value: u8) {
        self.cells.insert(addr & 0x00FF_FFFF, value);
    }

    /// Apply a suite-supplied `[address, value]` pair.
    pub fn set(&mut self, addr: u32, value: u32) {
        self.set_byte(addr, value as u8);
    }

    /// Read a byte back, for assertions.
    pub fn get(&self, addr: u32) -> Option<u8> {
        self.cells.get(&(addr & 0x00FF_FFFF)).copied()
    }
}

impl Bus for SparseBus {
    fn read8(&mut self, addr: u32) -> u8 {
        self.cells.get(&(addr & 0x00FF_FFFF)).copied().unwrap_or(0)
    }

    fn write8(&mut self, addr: u32, value: u8) {
        // Writes outside the declared set are recorded too: the suite declares
        // expected results for them, and overwriting here would mask a bug.
        self.cells.insert(addr & 0x00FF_FFFF, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_accesses_compose_from_bytes_big_endian() {
        let mut bus = SparseBus::new();
        bus.write32(0x1000, 0x1234_5678);
        // The 68000 bus is big-endian: the most significant byte is lowest.
        assert_eq!(bus.get(0x1000), Some(0x12));
        assert_eq!(bus.get(0x1003), Some(0x78));
        assert_eq!(bus.read32(0x1000), 0x1234_5678);
    }
}
