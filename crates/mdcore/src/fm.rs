//! Yamaha YM2612, the Mega Drive's FM sound chip.
//!
//! The 68000 reaches it through 0xA04000-0xA04003: ports 0 and 2 are the
//! register file, ports 1 and 3 are the data ports, and the port A address
//! latch is set once and used for a pair of writes.
//!
//! This is the register-and-status half only. Voice generation lives in
//! `ym2612` once it is needed for audio; until then the chip accepts its
//! programming and reports its status register so the Z80 sound driver runs
//! against something with the right handshake behaviour.

/// Number of addressable YM2612 registers.
const REGISTERS: usize = 256;

/// Status-register bits the Z80 driver polls.
const BUSY: u8 = 0x80;
const TIMER_A_OVERFLOW: u8 = 0x04;
const TIMER_B_OVERFLOW: u8 = 0x01;

#[derive(Debug, Clone)]
pub struct Fm {
    pub registers: [u8; REGISTERS],
    /// Latched by a write to port A, used by the next port B write.
    pub port_a: u8,
    /// Status register: busy plus the timer overflow flags.
    pub status: u8,
    /// Timer A period written by the driver.
    pub timer_a: u16,
    /// Timer B period written by the driver.
    pub timer_b: u8,
    /// Incremented on every write to port A, which is what the drivers use as
    /// a cheap entropy source for the noise LFSR.
    pub lfsr: u16,
    /// Total register writes. The Z80 never reads a meaningful status back
    /// from an FM register port, so a driver that appears to stall is either
    /// not writing at all or writing to the wrong address - and this count
    /// separates those two immediately.
    pub write_count: usize,
    /// Per-register write counts, addresses latched on port A.
    ///
    /// This is what separates a driver's *patch* from its *sequencing*. An
    /// instrument's envelope rates and operator levels are written once at
    /// startup and never again; the note data - which channel is sounding, at
    /// what frequency - is rewritten constantly. The same register file holds
    /// both, and the only way to tell them apart without re-reading the driver
    /// is to count how often each address is touched.
    pub reg_writes: [u32; REGISTERS],
    /// The value each address was last given, so the live note table can be
    /// read straight out of the chip.
    pub reg_last: [u8; REGISTERS],
    /// Writes per hardware port: 0/2 latch an address, 1/3 carry data.
    pub port_writes: [u32; 4],
}

impl Default for Fm {
    fn default() -> Self {
        Self::new()
    }
}

impl Fm {
    pub fn new() -> Self {
        Self {
            registers: [0; REGISTERS],
            port_a: 0,
            status: TIMER_A_OVERFLOW | TIMER_B_OVERFLOW,
            timer_a: 0,
            timer_b: 0,
            lfsr: 1,
            write_count: 0,
            reg_writes: [0; REGISTERS],
            reg_last: [0; REGISTERS],
            port_writes: [0; 4],
        }
    }

    /// Write to port A (address latch), port B (data) or the memory-mode
    /// registers on the even ports.
    ///
    /// `port` is the low two bits of the chip address.
    pub fn write(&mut self, port: usize, value: u8) {
        self.write_count += 1;
        if port < 4 {
            self.port_writes[port] += 1;
        }
        match port {
            0 | 2 => {
                self.port_a = value & 0x3F;
                // The address write is also visible as a write to the register
                // it selects, which is what memory-mode writes do.
                self.registers[self.port_a as usize] = value;
                self.lfsr = ((self.lfsr << 1) as u16 ^ if self.lfsr & 0x8000 != 0 { 1 } else { 0 })
                    & 0xFFFF;
            }
            1 | 3 => {
                let index = self.port_a as usize;
                self.registers[index] = value;
                self.reg_writes[index] += 1;
                self.reg_last[index] = value;
                match index {
                    // Timer A period, 16-bit across two writes.
                    0x24 => self.timer_a = ((self.timer_a as u16 & 0xFF) << 8) | value as u16,
                    0x25 => self.timer_a = (self.timer_a & 0xFF00) | value as u16,
                    // Timer B control and period.
                    0x26 => {
                        self.timer_b = value;
                        self.set_flag(BUSY, false);
                    }
                    0x27 => self.status &= !BUSY,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Read from port B, which returns the status register.
    pub fn read(&mut self, port: usize) -> u8 {
        if port & 1 == 1 {
            let status = self.status;
            // Reading clears the timer overflow flags, which is what drivers
            // rely on to synchronise with the timers.
            self.status &= !(TIMER_A_OVERFLOW | TIMER_B_OVERFLOW);
            return status;
        }
        0xFF
    }

    fn set_flag(&mut self, bit: u8, value: bool) {
        if value {
            self.status |= bit;
        } else {
            self.status &= !bit;
        }
    }

    /// Advance the timers by `cycles` master clocks.
    ///
    /// Only the overflow flags are modelled; the voice synthesis that consumes
    /// them is not part of this chip yet.
    pub fn tick(&mut self, _cycles: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_a_latches_the_register_address() {
        let mut fm = Fm::new();
        fm.write(0, 0x22);
        assert_eq!(fm.port_a, 0x22);
        fm.write(1, 0x08);
        assert_eq!(fm.registers[0x22], 0x08);
    }

    #[test]
    fn reading_status_clears_the_timer_overflows() {
        let mut fm = Fm::new();
        assert!(fm.read(1) & TIMER_A_OVERFLOW != 0);
        assert_eq!(fm.read(1) & TIMER_A_OVERFLOW, 0);
    }
}
