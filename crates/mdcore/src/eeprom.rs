//! Serial (I2C) EEPROM used by a set of cartridges for saves, following the
//! reference's state machine: START/STOP detection, 7-bit (X24C01) or device
//! address plus 8/16-bit word address phases, page writes and sequential reads.
//! The memory array is the cartridge's 64 KiB backup buffer.
use serde::{Deserialize, Serialize};

/// Chip characteristics: address bits (7 = X24C01 mode 1), array size mask and
/// page-write mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub address_bits: u8,
    pub size_mask: u16,
    pub page_mask: u16,
}

pub const X24C01: Spec = Spec { address_bits: 7, size_mask: 0x7F, page_mask: 0x03 };
pub const X24C02: Spec = Spec { address_bits: 8, size_mask: 0xFF, page_mask: 0x03 };
pub const C24C02: Spec = Spec { address_bits: 8, size_mask: 0xFF, page_mask: 0x07 };
pub const C24C04: Spec = Spec { address_bits: 8, size_mask: 0x1FF, page_mask: 0x0F };
pub const C24C08: Spec = Spec { address_bits: 8, size_mask: 0x3FF, page_mask: 0x0F };
pub const C24C16: Spec = Spec { address_bits: 8, size_mask: 0x7FF, page_mask: 0x0F };
pub const C24C65: Spec = Spec { address_bits: 16, size_mask: 0x1FFF, page_mask: 0x3F };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum State {
    StandBy,
    WaitStop,
    WordAddress7,
    DeviceAddress,
    WordAddressHigh,
    WordAddressLow,
    WriteData,
    ReadData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct I2c {
    pub spec: Spec,
    sda: bool,
    scl: bool,
    old_sda: bool,
    old_scl: bool,
    cycles: u8,
    rw: bool,
    device_address: u16,
    word_address: u16,
    buffer: u8,
    state: State,
}

impl I2c {
    pub fn new(spec: Spec) -> Self {
        Self {
            spec,
            sda: true,
            scl: true,
            old_sda: true,
            old_scl: true,
            cycles: 0,
            rw: false,
            device_address: 0,
            word_address: 0,
            buffer: 0,
            state: State::StandBy,
        }
    }

    fn start(&mut self) {
        if self.old_scl && self.scl && self.old_sda && !self.sda {
            self.cycles = 0;
            if self.spec.address_bits == 7 {
                self.word_address = 0;
                self.state = State::WordAddress7;
            } else {
                self.device_address = 0;
                self.state = State::DeviceAddress;
            }
        }
    }

    fn stop(&mut self) {
        if self.old_scl && self.scl && !self.old_sda && self.sda {
            self.state = State::StandBy;
        }
    }

    /// Drive SDA/SCL and advance the protocol; writes land in `memory`.
    pub fn set_lines(&mut self, sda: bool, scl: bool, memory: &mut [u8]) {
        self.sda = sda;
        self.scl = scl;
        let falling = self.old_scl && !self.scl;
        let rising = !self.old_scl && self.scl;
        let bit = u16::from(self.sda);
        match self.state {
            State::StandBy => self.start(),
            State::WaitStop => self.stop(),
            State::WordAddress7 => {
                self.start();
                self.stop();
                if falling {
                    if self.cycles < 9 {
                        self.cycles += 1;
                    } else {
                        self.cycles = 1;
                        self.state = if self.rw { State::ReadData } else { State::WriteData };
                        self.buffer = 0;
                    }
                } else if rising {
                    if self.cycles < 8 {
                        self.word_address |= bit << (7 - self.cycles);
                    } else if self.cycles == 8 {
                        self.rw = self.sda;
                    }
                }
            }
            State::DeviceAddress => {
                self.start();
                self.stop();
                if falling {
                    if self.cycles < 9 {
                        self.cycles += 1;
                    } else {
                        self.device_address <<= self.spec.address_bits;
                        self.cycles = 1;
                        if self.rw {
                            self.state = State::ReadData;
                        } else {
                            self.word_address = 0;
                            self.state = if self.spec.address_bits == 16 {
                                State::WordAddressHigh
                            } else {
                                State::WordAddressLow
                            };
                        }
                    }
                } else if rising {
                    if self.cycles > 4 && self.cycles < 8 {
                        self.device_address |= bit << (7 - self.cycles);
                    } else if self.cycles == 8 {
                        self.rw = self.sda;
                    }
                }
            }
            State::WordAddressHigh => {
                self.start();
                self.stop();
                if falling {
                    if self.cycles < 9 {
                        self.cycles += 1;
                    } else {
                        self.cycles = 1;
                        self.state = State::WordAddressLow;
                    }
                } else if rising && self.cycles < 9 {
                    if u32::from(self.spec.size_mask) < (1 << (16 - self.cycles)) {
                        self.device_address >>= 1;
                    } else {
                        self.word_address |= bit << (16 - self.cycles);
                    }
                }
            }
            State::WordAddressLow => {
                self.start();
                self.stop();
                if falling {
                    if self.cycles < 9 {
                        self.cycles += 1;
                    } else {
                        self.cycles = 1;
                        self.state = State::WriteData;
                        self.buffer = 0;
                    }
                } else if rising && self.cycles < 9 {
                    if u32::from(self.spec.size_mask) < (1 << (8 - self.cycles)) {
                        self.device_address >>= 1;
                    } else {
                        self.word_address |= bit << (8 - self.cycles);
                    }
                }
            }
            State::ReadData => {
                self.start();
                self.stop();
                if falling {
                    self.cycles = if self.cycles < 9 { self.cycles + 1 } else { 1 };
                } else if rising && self.cycles == 9 {
                    if self.sda {
                        self.state = State::WaitStop;
                    } else {
                        self.word_address = (self.word_address + 1) & self.spec.size_mask;
                    }
                }
            }
            State::WriteData => {
                self.start();
                self.stop();
                if falling {
                    self.cycles = if self.cycles < 9 { self.cycles + 1 } else { 1 };
                } else if rising {
                    if self.cycles < 9 {
                        self.buffer |= u8::from(self.sda) << (8 - self.cycles);
                    } else {
                        let at = usize::from(self.device_address | self.word_address);
                        memory[at & 0xFFFF] = self.buffer;
                        self.buffer = 0;
                        let page = self.spec.page_mask;
                        self.word_address = (self.word_address & !page) | ((self.word_address + 1) & page);
                    }
                }
            }
        }
        self.old_scl = self.scl;
        self.old_sda = self.sda;
    }

    /// Current (SDA, SCL) input levels.
    pub fn lines(&self) -> (bool, bool) {
        (self.sda, self.scl)
    }

    /// SDA as the chip drives it (data bit, ACK low) or the latched input.
    pub fn sda_out(&self, memory: &[u8]) -> bool {
        if self.state == State::ReadData {
            if self.cycles < 9 {
                let at = usize::from(self.device_address | self.word_address) & 0xFFFF;
                return (memory[at] >> (8 - self.cycles)) & 1 != 0;
            }
        } else if self.cycles == 9 {
            return false;
        }
        self.sda
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bit-bang one byte MSB first, then clock the ACK slot; returns ACK level.
    fn send(chip: &mut I2c, memory: &mut [u8], byte: u8) -> bool {
        for i in (0..8).rev() {
            let sda = byte >> i & 1 != 0;
            chip.set_lines(sda, false, memory);
            chip.set_lines(sda, true, memory);
            chip.set_lines(sda, false, memory);
        }
        chip.set_lines(true, false, memory);
        chip.set_lines(true, true, memory);
        let ack = chip.sda_out(memory);
        chip.set_lines(true, false, memory);
        ack
    }

    fn start(chip: &mut I2c, memory: &mut [u8]) {
        chip.set_lines(true, true, memory);
        chip.set_lines(false, true, memory);
        chip.set_lines(false, false, memory);
    }

    fn stop(chip: &mut I2c, memory: &mut [u8]) {
        chip.set_lines(false, false, memory);
        chip.set_lines(false, true, memory);
        chip.set_lines(true, true, memory);
    }

    fn receive(chip: &mut I2c, memory: &mut [u8], last: bool) -> u8 {
        let mut value = 0;
        for _ in 0..8 {
            chip.set_lines(true, true, memory);
            value = value << 1 | u8::from(chip.sda_out(memory));
            chip.set_lines(true, false, memory);
        }
        chip.set_lines(last, false, memory);
        chip.set_lines(last, true, memory);
        chip.set_lines(last, false, memory);
        value
    }

    #[test]
    fn x24c01_mode1_writes_a_page_and_reads_it_back_sequentially() {
        let mut memory = vec![0xFF; 0x10000];
        let mut chip = I2c::new(X24C01);
        start(&mut chip, &mut memory);
        // 7-bit word address 5, R/W = 0 (write).
        assert!(!send(&mut chip, &mut memory, 5 << 1));
        assert!(!send(&mut chip, &mut memory, 0xAB));
        assert!(!send(&mut chip, &mut memory, 0xCD));
        stop(&mut chip, &mut memory);
        assert_eq!(&memory[5..7], &[0xAB, 0xCD]);
        start(&mut chip, &mut memory);
        send(&mut chip, &mut memory, 5 << 1 | 1);
        assert_eq!(receive(&mut chip, &mut memory, false), 0xAB);
        assert_eq!(receive(&mut chip, &mut memory, true), 0xCD);
        stop(&mut chip, &mut memory);
    }

    #[test]
    fn c24c65_uses_a_16_bit_word_address() {
        let mut memory = vec![0xFF; 0x10000];
        let mut chip = I2c::new(C24C65);
        start(&mut chip, &mut memory);
        send(&mut chip, &mut memory, 0xA0); // device address, write
        send(&mut chip, &mut memory, 0x12);
        send(&mut chip, &mut memory, 0x34);
        send(&mut chip, &mut memory, 0x5A);
        stop(&mut chip, &mut memory);
        assert_eq!(memory[0x1234], 0x5A);
    }
}
