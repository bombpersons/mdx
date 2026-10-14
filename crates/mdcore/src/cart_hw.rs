//! Cartridge-side hardware, following Genesis Plus GX's default handling:
//! battery RAM (SRAM) declared in the header, the $A130F1 SRAM/ROM switch with
//! write protection, the $A130F3..$A130FF 512 KiB bank registers and the
//! $A13000..$A1305F multi-game 64 KiB mapper (not applied), serial EEPROM
//! boards (EA, Sega, Acclaim 16M/32M, Codemasters) and J-Cart controller ports.
//! Other title-specific exceptions in the reference's database are not modeled.
use crate::cartridge::Cartridge;
use crate::eeprom::{self, I2c, Spec};
use serde::{Deserialize, Serialize};

/// Serial EEPROM / J-Cart board wiring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Board {
    #[default]
    None,
    /// EA: EEPROM at $200000-$3FFFFF, SCL D6, SDA D7.
    Ea,
    /// Sega: EEPROM at $200000-$3FFFFF, SCL D1, SDA D0.
    Sega,
    /// Acclaim 16M: any write drives SCL D1/SDA D0; SDA reads on D1.
    Acclaim16,
    /// Acclaim 32M: byte writes drive SDA (odd) or SCL (even) from D0; word
    /// writes switch $200000-$2FFFFF reads between ROM (D0 set) and EEPROM.
    Acclaim32,
    /// Codemasters: EEPROM writes at $300000-$37FFFF (SCL D1, SDA D0); reads
    /// and J-Cart ports at $380000-$3FFFFF (SDA on D7).
    Codemasters { jcart: bool },
}

/// (product id, stack pointer, header checksum, board, chip); 0 = any.
const EEPROM_GAMES: &[(&str, u32, u16, Board, Option<Spec>)] = &[
    ("T-50176", 0, 0, Board::Ea, Some(eeprom::X24C01)),
    ("T-50396", 0, 0, Board::Ea, Some(eeprom::X24C01)),
    ("T-50446", 0, 0, Board::Ea, Some(eeprom::X24C01)),
    ("T-50516", 0, 0, Board::Ea, Some(eeprom::X24C01)),
    ("T-50606", 0, 0, Board::Ea, Some(eeprom::X24C01)),
    (" T-12046", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    (" T-12053", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("MK-1215", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("MK-1228", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("G-5538", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("PR-1993", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    (" G-4060", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("00001211", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("00004076", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("G-4524", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("00054503", 0, 0, Board::Sega, Some(eeprom::X24C01)),
    ("T-81033", 0, 0, Board::Acclaim16, Some(eeprom::X24C02)),
    ("T-081326", 0, 0, Board::Acclaim16, Some(eeprom::X24C02)),
    ("T-081276", 0, 0, Board::Acclaim32, Some(eeprom::C24C02)),
    ("T-81406", 0, 0, Board::Acclaim32, Some(eeprom::C24C04)),
    ("T-081586", 0, 0, Board::Acclaim32, Some(eeprom::C24C16)),
    ("T-81476", 0, 0, Board::Acclaim32, Some(eeprom::C24C65)),
    ("T-81576", 0, 0, Board::Acclaim32, Some(eeprom::C24C65)),
    ("T-120106", 0, 0, Board::Codemasters { jcart: false }, Some(eeprom::C24C08)),
    ("00000000", 0x444e_4c44, 0x168B, Board::Codemasters { jcart: true }, Some(eeprom::C24C08)),
    ("00000000", 0x444e_4c44, 0x165E, Board::Codemasters { jcart: true }, Some(eeprom::C24C16)),
    ("T-120096", 0, 0, Board::Codemasters { jcart: true }, Some(eeprom::C24C16)),
    ("T-120146", 0, 0, Board::Codemasters { jcart: false }, Some(eeprom::C24C65)),
    ("00000000", 0xffff_fffc, 0x168B, Board::Codemasters { jcart: true }, None),
    ("00000000", 0xffff_fffc, 0x165E, Board::Codemasters { jcart: true }, None),
    ("T-120066", 0, 0, Board::Codemasters { jcart: true }, None),
    ("T-123456", 0, 0, Board::Codemasters { jcart: true }, None),
    ("XXXXXXXX", 0, 0xDF39, Board::Codemasters { jcart: true }, None),
];

/// A cartridge-window access handled by EEPROM/J-Cart wiring.
pub enum Special {
    Value(u16),
    /// Unconnected byte lane: the 68000's open bus.
    OpenBus,
}

const SRAM_SIZE: usize = 0x10000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CartHw {
    /// ROM offset mapped into each 64 KiB bank of $000000..$3FFFFF.
    banks: Vec<u32>,
    /// 64 KiB backup RAM image, initialized to 0xFF; empty without SRAM.
    pub sram: Vec<u8>,
    pub sram_start: u32,
    pub sram_end: u32,
    /// SRAM currently occupies the 64 KiB bank containing `sram_start`.
    pub sram_mapped: bool,
    pub sram_writable: bool,
    /// ROM offset restored at $200000 when SRAM is switched out.
    saved_bank: u32,
    #[serde(default)]
    pub board: Board,
    /// Serial EEPROM state; its array is `sram`.
    #[serde(default)]
    pub eeprom: Option<I2c>,
    /// Acclaim 32M: $200000-$2FFFFF reads come from ROM (else the EEPROM).
    #[serde(default)]
    acclaim_rom_reads: bool,
    /// J-Cart TH output to both extra controller ports.
    #[serde(default)]
    pub jcart_th: bool,
    /// Banks [start, end) where backup RAM is always mapped (Barkley Shut Up
    /// and Jam 2: whole upper 2 MiB; Triple Play: $200000-$2FFFFF).
    #[serde(default)]
    pub extra_sram_banks: (u8, u8),
    /// Reference cartridge database hardware (protection registers, mappers).
    #[serde(default)]
    pub hw: crate::cart_db::Hardware,
    /// 32 KiB mapper selection for banks 0-15 (custom_regs_w), when active.
    #[serde(default)]
    bank32: Option<u16>,
    /// Banks still showing the Realtec board's 8 KiB boot ROM.
    #[serde(default)]
    realtec_boot: u64,
    /// The cartridge asserted /VRES (Realtec): reset the 68000 after this access.
    #[serde(skip)]
    pub reset_request: bool,
    /// Banks whose reads are open bus / backup RAM (SF boards).
    #[serde(default)]
    open_bus_banks: u64,
    #[serde(default)]
    sram_banks: u64,
    #[serde(default)]
    pub sf: crate::cart_db::SfBoard,
    /// SF board bankswitch writes disabled until hard reset.
    #[serde(default)]
    sf_locked: bool,
    /// Super Mario World 64 board: ROM offsets of the $60/$68 and $61/$69
    /// windows (registers live in backup RAM bytes 0-8).
    #[serde(default)]
    pub smw64: Option<(u32, u32)>,
}

/// What a cartridge-window read sees.
pub enum CartRead {
    Byte(u8),
    Rom(u32),
    OpenBus,
}

fn header_long(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(std::array::from_fn(|i| data.get(offset + i).copied().unwrap_or(0xFF)))
}

fn header_text(data: &[u8], range: std::ops::Range<usize>) -> String {
    data.get(range).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
}

impl CartHw {
    pub fn new(cart: &Cartridge) -> Self {
        let mut hw = Self::new_board(cart);
        let data = &cart.data;
        let header = u16::from_be_bytes([data.get(0x18E).copied().unwrap_or(0), data.get(0x18F).copied().unwrap_or(0)]);
        let real = data.get(0x200..).unwrap_or(&[]).chunks(2).fold(0u16, |sum, w| {
            sum.wrapping_add(u16::from_be_bytes([w[0], *w.get(1).unwrap_or(&0)]))
        });
        if let Some(entry) = crate::cart_db::lookup(header, real) {
            hw.hw = crate::cart_db::Hardware::from_entry(entry);
            if hw.hw.realtec {
                hw.realtec_boot = u64::MAX;
            }
        } else if data.get(8..12) == Some(&[0x60, 0x00, 0x01, 0xF6]) && real == 0xF894 {
            // Super Mario World 64: lower 512 KiB mirrored at $080000, the
            // rest of the cartridge window unmapped, custom hardware at $6x.
            for (i, bank) in hw.banks.iter_mut().enumerate().take(0x10).skip(0x08) {
                *bank = ((i as u32) & 7) << 16;
            }
            hw.open_bus_banks = !0u64 << 0x10;
            hw.smw64 = Some((0x0F_0000, 0x0F_0000));
            if hw.sram.is_empty() {
                hw.sram = vec![0xFF; SRAM_SIZE];
            }
        } else {
            let rom_type = header_text(data, 0x180..0x182);
            let product = header_text(data, 0x182..0x18E);
            use crate::cart_db::{SfBoard, TimeRead, TimeWrite};
            if rom_type.contains("SF") && product.contains("001") {
                hw.sf = SfBoard::Sf001;
                hw.hw.time_w = TimeWrite::Unused;
            } else if rom_type.contains("SF") && product.contains("002") {
                hw.sf = SfBoard::Sf002;
                hw.hw.time_w = TimeWrite::Unused;
            } else if rom_type.contains("SF") && product.contains("004") {
                hw.sf = SfBoard::Sf004;
                hw.hw.time_r = TimeRead::Sf004;
                hw.hw.time_w = TimeWrite::Unused;
                // First 256 KiB bank mirrored into $000000-$1FFFFF; SRAM
                // ($200000-$2FFFFF) disabled and $300000-$3FFFFF unmapped.
                for (i, bank) in hw.banks.iter_mut().enumerate().take(0x20) {
                    *bank = ((i as u32) & 3) << 16;
                }
                hw.sram_mapped = false;
                hw.open_bus_banks = !0u64 << 0x20;
                if hw.sram.is_empty() {
                    hw.sram = vec![0xFF; SRAM_SIZE];
                }
            }
        }
        hw
    }

    fn new_board(cart: &Cartridge) -> Self {
        let data = &cart.data;
        let rom_size = data.len() as u32;
        let international = header_text(data, 0x150..0x180);
        let product = header_text(data, 0x180..0x18E);
        let mut sram = None;
        let declared = data.get(0x1B0..0x1B2) == Some(b"RA");
        if declared {
            let mut start = header_long(data, 0x1B4) & 0xFF_FFFF;
            let mut end = header_long(data, 0x1B8) & 0xFF_FFFF;
            if start == 0xFF_0000 {
                // Work RAM declared as volatile external RAM: no backup RAM.
            } else {
                if product.contains("T-26013") {
                    (start, end) = (0x20_0001, 0x20_3FFF); // Psy-O-Blade
                } else if start >= 0x80_0000 {
                    (start, end) = (0x20_0000, 0x20_FFFF);
                } else if start > end || end - start >= 0x1_0000 {
                    end = start + 0xFFFF;
                }
                sram = Some((start, end));
            }
        } else if data.get(0x1B0..0x1B2) != Some(b"FL") {
            // No declaration: the reference's sram_init table, then 64 KiB
            // static RAM at $200000 for ROMs up to 2 MiB.
            let checksum = u16::from_be_bytes([data.get(0x18E).copied().unwrap_or(0), data.get(0x18F).copied().unwrap_or(0)]);
            let real = data.get(0x200..).unwrap_or(&[]).chunks(2).fold(0u16, |sum, w| {
                sum.wrapping_add(u16::from_be_bytes([w[0], *w.get(1).unwrap_or(&0)]))
            });
            let rom_type = header_text(data, 0x180..0x182);
            sram = if product.contains("T-50086") || product.contains("T-50286") {
                Some((0x20_0001, 0x20_3FFF)) // PGA Tour Golf, Buck Rogers
            } else if product.contains("ACLD007") {
                Some((0x20_0001, 0x20_0FFF)) // Winter Challenge
            } else if matches!(real, 0xAEAA | 0x8DBA) && checksum == 0x8104 {
                Some((0x40_0001, 0x40_FFFF)) // Xin Qigai Wangzi
            } else if checksum == 0 && real == 0x1F7F && data.get(0x801B0..0x801B2) == Some(b"RA") {
                Some((0x20_0001, 0x20_3FFF)) // Radica Sensible Soccer Plus
            } else if rom_type.contains("SF") && product.get(2..).unwrap_or("").contains("001") {
                Some(if checksum == 0x3E08 { (0x3C_0001, 0x3C_FFFF) } else { (0x40_0001, 0x40_FFFF) })
            } else if rom_type.contains("SF") && product.get(2..).unwrap_or("").contains("004") {
                Some((0x20_0001, 0x20_3FFF))
            } else if international.contains("SONIC & KNUCKLES") {
                (rom_size == 0x40_0000).then_some((0x20_0001, 0x20_3FFF))
            } else if international.contains("COLOCODX")
                || product.contains("T-113016")
                || international.contains("SONIC THE HEDGEHOG 2")
            {
                None // flash (unsupported), Pugsy, Sonic 2
            } else if rom_size <= 0x20_0000 {
                Some((0x20_0000, 0x20_FFFF))
            } else {
                None
            };
        }
        let mut banks: Vec<u32> = (0..64u32).map(|bank| bank << 16).collect();
        // QuackShot Rev A 512 KiB dump: VA18/VA19 unconnected, VA20 drives
        // A19, VA21 unconnected (reference md_cart_init).
        let header_checksum = u16::from_be_bytes([data.get(0x18E).copied().unwrap_or(0), data.get(0x18F).copied().unwrap_or(0)]);
        if product.contains("00004054-01") && rom_size == 0x8_0000 && header_checksum == 0xA4B3 {
            for (bank, offset) in banks.iter_mut().enumerate() {
                let i = bank as u32 & 0x1F;
                *offset = if i < 0x10 { (i & 3) << 16 } else { 0x4_0000 + ((i & 3) << 16) };
            }
        }
        // Serial EEPROM / J-Cart boards (reference database, then header).
        let checksum = u16::from_be_bytes([data.get(0x18E).copied().unwrap_or(0), data.get(0x18F).copied().unwrap_or(0)]);
        let sp = header_long(data, 0);
        let range = sram.map_or(0, |(start, end): (u32, u32)| end.wrapping_sub(start));
        let mut board = Board::None;
        let mut chip = None;
        if let Some((_, _, _, b, spec)) = EEPROM_GAMES.iter().rev().find(|(id, game_sp, chk, _, _)| {
            product.contains(id)
                // A leading space marks titles with known SRAM-patched hacks.
                && !(id.starts_with(' ') && range > 2)
                && (*chk == 0 || *chk == checksum)
                && (*game_sp == 0 || *game_sp == sp)
        }) {
            board = *b;
            chip = *spec;
        } else if declared && (data.get(0x1B2) == Some(&0xE8) || range < 2) {
            board = Board::Sega;
            chip = Some(eeprom::X24C01);
        }
        if chip.is_some() {
            return Self {
                banks,
                sram: if chip.is_some() { vec![0xFF; SRAM_SIZE] } else { Vec::new() },
                sram_start: 0,
                sram_end: 0,
                sram_mapped: false,
                sram_writable: false,
                saved_bank: 0x20_0000,
                board,
                eeprom: chip.map(I2c::new),
                acclaim_rom_reads: true,
                jcart_th: true,
                extra_sram_banks: (0, 0),
                hw: Default::default(),
                bank32: None,
                realtec_boot: 0,
                reset_request: false,
                open_bus_banks: 0,
                sram_banks: 0,
                sf: Default::default(),
                sf_locked: false,
                smw64: None,
            };
        }
        let Some((sram_start, sram_end)) = sram else {
            return Self {
                banks,
                sram: Vec::new(),
                sram_start: 0,
                sram_end: 0,
                sram_mapped: false,
                sram_writable: false,
                saved_bank: 0x20_0000,
                // J-Cart-only boards keep the ordinary backup RAM rules.
                board,
                eeprom: None,
                acclaim_rom_reads: true,
                jcart_th: true,
                extra_sram_banks: (0, 0),
                hw: Default::default(),
                bank32: None,
                realtec_boot: 0,
                reset_request: false,
                open_bus_banks: 0,
                sram_banks: 0,
                sf: Default::default(),
                sf_locked: false,
                smw64: None,
            };
        };
        // Mapped at power-on unless it overlaps the ROM (then $A130F1 maps it).
        let mapped = sram_start >= rom_size && !international.contains("SONIC THE HEDGEHOG 3");
        let mut extra_sram_banks = (0, 0);
        if sram_start >= rom_size && product.contains("T-119186") {
            extra_sram_banks = (0x21, 0x40);
        } else if sram_start < rom_size && (product.contains("T-172026") || product.contains("T-172116")) {
            extra_sram_banks = (0x20, 0x30);
            if data.get(0x20_0000).is_some_and(|b| *b != 0xFF) {
                for (bank, offset) in banks.iter_mut().enumerate().skip(0x30) {
                    *offset = ((bank as u32) - 0x10) << 16;
                }
            }
        }
        Self {
            banks,
            sram: vec![0xFF; SRAM_SIZE],
            sram_start,
            sram_end,
            sram_mapped: mapped,
            sram_writable: mapped,
            saved_bank: 0x20_0000,
            board,
            eeprom: None,
            acclaim_rom_reads: true,
            jcart_th: true,
            extra_sram_banks,
            hw: Default::default(),
            bank32: None,
            realtec_boot: 0,
            reset_request: false,
            open_bus_banks: 0,
            sram_banks: 0,
            sf: Default::default(),
            sf_locked: false,
            smw64: None,
        }
    }

    /// Line bit positions: (SCL in, SDA in, SDA out).
    fn bits(&self) -> (u32, u32, u32) {
        match self.board {
            Board::Ea => (6, 7, 7),
            Board::Sega => (1, 0, 0),
            Board::Acclaim16 => (1, 0, 1),
            Board::Acclaim32 => (0, 0, 0),
            Board::Codemasters { .. } => (1, 0, 7),
            Board::None => (0, 0, 0),
        }
    }

    fn sda_out(&self) -> u16 {
        let level = match &self.eeprom {
            Some(chip) => chip.sda_out(&self.sram),
            // J-Cart boards without EEPROM: the latched (pulled-up) line.
            None => true,
        };
        u16::from(level) << self.bits().2
    }

    fn drive(&mut self, sda: bool, scl: bool) {
        if let Some(chip) = self.eeprom.as_mut() {
            chip.set_lines(sda, scl, &mut self.sram);
        }
    }

    fn generic_write(&mut self, data: u16) {
        let (scl, sda, _) = self.bits();
        self.drive(data >> sda & 1 != 0, data >> scl & 1 != 0);
    }

    pub fn has_jcart(&self) -> bool {
        matches!(self.board, Board::Codemasters { jcart: true })
    }

    /// EEPROM/J-Cart read at a cartridge-window address, if wired there.
    /// `pads` is the J-Cart port word (port 2 in D7-D0, port 3 D5-D0 in D13-D8).
    pub fn special_read(&self, address: u32, word: bool, pads: u16) -> Option<Special> {
        let bank = (address >> 16) & 0x3F;
        let generic = |hw: &Self| {
            if word || address & 1 != 0 {
                Special::Value(hw.sda_out())
            } else {
                Special::OpenBus
            }
        };
        match self.board {
            Board::None => None,
            Board::Ea | Board::Sega | Board::Acclaim16 if bank >= 0x20 => Some(generic(self)),
            Board::Acclaim32 if (0x20..0x30).contains(&bank) && !self.acclaim_rom_reads => Some(generic(self)),
            Board::Codemasters { jcart } if bank >= 0x38 => Some(if !jcart {
                generic(self)
            } else if word {
                Special::Value(self.sda_out() | pads)
            } else if address & 1 != 0 {
                Special::Value(self.sda_out() | (pads & 0x7F))
            } else {
                Special::Value(pads >> 8)
            }),
            _ => None,
        }
    }

    /// EEPROM/J-Cart write; true when the wiring took it (ROM writes ignored).
    pub fn special_write(&mut self, address: u32, data: u16, word: bool) -> bool {
        let bank = (address >> 16) & 0x3F;
        match self.board {
            Board::None => false,
            Board::Ea | Board::Sega if bank >= 0x20 => {
                if word || address & 1 != 0 {
                    self.generic_write(data);
                }
                true
            }
            Board::Acclaim16 if bank >= 0x20 => {
                self.generic_write(data);
                true
            }
            Board::Acclaim32 if (0x20..0x30).contains(&bank) => {
                if word {
                    self.acclaim_rom_reads = data & 1 != 0;
                } else if let Some(chip) = &self.eeprom {
                    let (mut sda, mut scl) = chip.lines();
                    if address & 1 != 0 {
                        sda = data & 1 != 0;
                    } else {
                        scl = data & 1 != 0;
                    }
                    self.drive(sda, scl);
                }
                true
            }
            Board::Codemasters { .. } if (0x30..0x38).contains(&bank) => {
                self.generic_write(data);
                true
            }
            Board::Codemasters { jcart } if bank >= 0x38 => {
                if jcart {
                    self.jcart_th = data & 1 != 0;
                }
                true
            }
            _ => false,
        }
    }

    pub fn has_sram(&self) -> bool {
        !self.sram.is_empty()
    }

    fn sram_bank(&self, address: u32) -> bool {
        self.sram_mapped && (address >> 16) & 0x3F == self.sram_start >> 16
    }

    fn extra_sram_bank(&self, address: u32) -> bool {
        let bank = ((address >> 16) & 0x3F) as u8;
        (self.extra_sram_banks.0..self.extra_sram_banks.1).contains(&bank)
    }

    /// Byte at a cartridge-window address, or `None` for ROM at `rom_offset`.
    /// Cartridge-window read including open-bus banks.
    pub fn read_kind(&self, address: u32) -> CartRead {
        let bank = (address >> 16) & 0x3F;
        if self.sram_banks & (1 << bank) != 0 {
            return CartRead::Byte(self.sram[(address & 0xFFFF) as usize]);
        }
        if self.open_bus_banks & (1 << bank) != 0 {
            return CartRead::OpenBus;
        }
        match self.read(address) {
            Ok(byte) => CartRead::Byte(byte),
            Err(offset) => CartRead::Rom(offset),
        }
    }

    /// The ROM offset a bank's base points at, ignoring read handlers (the
    /// reference's open-bus and instruction-prefetch view).
    pub fn raw_offset(&self, address: u32) -> u32 {
        let bank = (address >> 16) & 0x3F;
        self.banks[bank as usize] + (address & 0xFFFF)
    }

    pub fn read(&self, address: u32) -> Result<u8, u32> {
        let bank = (address >> 16) & 0x3F;
        if self.realtec_boot & (1 << bank) != 0 {
            // 8 KiB boot ROM (the image's last 8 KiB of its first 512 KiB).
            return Err(0x7_E000 + (address & 0x1FFF));
        }
        if self.sram_bank(address) || self.extra_sram_bank(address) {
            return Ok(self.sram[(address & 0xFFFF) as usize]);
        }
        if let Some(data) = self.bank32.filter(|_| bank < 0x10) {
            let half = (address >> 15) & 1;
            let base = (bank << 16) | (((u32::from(data) | half) & 0x3F) << 15);
            return Err(base + (address & 0x7FFF));
        }
        Err(self.banks[((address >> 16) & 0x3F) as usize] + (address & 0xFFFF))
    }

    /// A byte write into the cartridge window; only mapped, writable SRAM takes it.
    /// SF board bankswitch registers (writes into bank 0, or anywhere for
    /// SF-002). Returns whether the board took the write.
    pub fn sf_write(&mut self, address: u32, data: u16) -> bool {
        use crate::cart_db::SfBoard;
        let address = address & 0xFF_FFFF;
        if self.sf == SfBoard::None || self.sf_locked || address >> 16 != 0 {
            return self.sram_banks & (1 << ((address >> 16) & 0x3F)) != 0 && {
                self.sram[(address & 0xFFFF) as usize] = data as u8;
                true
            };
        }
        let all = u64::MAX;
        match self.sf {
            SfBoard::None => return false,
            SfBoard::Sf001 => {
                if (address >> 8) & 0xF == 0xE {
                    if data & 0x40 != 0 {
                        self.open_bus_banks = all;
                        self.sram_banks = 0;
                        for (i, bank) in self.banks.iter_mut().enumerate() {
                            *bank = (i as u32) << 16;
                        }
                    } else if data & 0x80 != 0 {
                        self.open_bus_banks = 0;
                        let sram_bank = (self.sram_start >> 16).min(0x40) as usize;
                        for (i, bank) in self.banks.iter_mut().enumerate() {
                            *bank = if i < 4 { (0x38 + i as u32) << 16 } else { (i as u32) << 16 };
                        }
                        self.sram_banks = if sram_bank < 0x40 { all & !((1u64 << sram_bank) - 1) } else { 0 };
                    } else {
                        self.open_bus_banks = 0;
                        self.sram_banks = 0;
                        for (i, bank) in self.banks.iter_mut().enumerate() {
                            *bank = (i as u32) << 16;
                        }
                    }
                    if data & 0x20 != 0 {
                        self.sf_locked = true;
                    }
                }
            }
            SfBoard::Sf002 => {
                for (i, bank) in self.banks.iter_mut().enumerate().take(0x3C).skip(0x20) {
                    *bank = if data & 0x80 != 0 { ((i as u32) & 0x1F) << 16 } else { (i as u32) << 16 };
                }
            }
            SfBoard::Sf004 => match (address >> 8) & 0xF {
                0xD => {
                    let sram = ((1u64 << 0x30) - 1) & !((1u64 << 0x20) - 1);
                    if data & 0x80 != 0 {
                        self.sram_banks |= sram;
                        self.open_bus_banks &= !sram;
                    } else {
                        self.sram_banks &= !sram;
                        self.open_bus_banks |= sram;
                    }
                }
                0xE => {
                    let low = (1u64 << 0x20) - 1;
                    if data & 0x20 != 0 {
                        self.open_bus_banks |= low;
                    } else if data & 0x40 != 0 {
                        let base = self.banks[0] >> 16;
                        for (i, bank) in self.banks.iter_mut().enumerate().take(0x14) {
                            *bank = ((base + i as u32) & 0x1F) << 16;
                        }
                        self.open_bus_banks = (self.open_bus_banks & !low) | (low & !((1u64 << 0x14) - 1));
                    } else {
                        let first = self.banks[0];
                        for (i, bank) in self.banks.iter_mut().enumerate().take(0x20) {
                            *bank = first + (((i as u32) & 3) << 16);
                        }
                        self.open_bus_banks &= !low;
                    }
                    if data & 0x80 == 0 {
                        self.sf_locked = true;
                    }
                }
                0xF => {
                    let base = ((u32::from(data) >> 4) & 7) << 2;
                    if self.banks[0] == self.banks[4] {
                        for (i, bank) in self.banks.iter_mut().enumerate().take(0x20) {
                            *bank = (base + ((i as u32) & 3)) << 16;
                        }
                    } else {
                        for (i, bank) in self.banks.iter_mut().enumerate().take(0x14) {
                            *bank = ((base + i as u32) & 0x1F) << 16;
                        }
                    }
                }
                _ => {}
            },
        }
        true
    }

    /// Super Mario World 64 hardware at $600000-$6FFFFF: a ROM offset for
    /// window reads, or a register value.
    pub fn smw64_read(&mut self, address: u32) -> Option<CartRead> {
        let (b60, b61) = self.smw64?;
        let address = address & 0xFF_FFFF;
        if address >> 20 != 6 {
            return None;
        }
        let bank = (address >> 16) & 0x0F;
        if bank & 7 < 4 {
            let base = match bank & 7 { 0 => b60, 1 => b61, _ => 0x0F_0000 };
            return Some(CartRead::Rom(base + (address & 0xFFFF)));
        }
        let s = &mut self.sram;
        let value = match (address >> 16) & 3 {
            2 => match (address >> 1) & 7 {
                0 => s[6],
                1 => s[6].wrapping_add(1),
                2 => s[7],
                3 => s[7].wrapping_add(1),
                n => s[8].wrapping_add((n - 4) as u8),
            },
            3 => {
                let mut data = if s[2] & 0x80 != 0 {
                    if s[5] & 0x40 != 0 { s[3] & s[4] } else { s[3] ^ 0xFF }
                } else {
                    0
                };
                if address & 2 != 0 {
                    data &= 0x7F;
                } else if s[5] & 0x80 != 0 {
                    if s[5] & 0x20 != 0 {
                        s[8] = (s[4] << 2) & 0xFC;
                    } else {
                        s[6] = (s[1] ^ (s[3] << 1)) & 0xFE;
                    }
                }
                data
            }
            _ => 0,
        };
        Some(CartRead::Byte(value))
    }

    /// Super Mario World 64 register writes ($600000-$6FFFFF).
    pub fn smw64_write(&mut self, address: u32, data: u16) -> bool {
        let Some((mut b60, mut b61)) = self.smw64 else { return false };
        let address = address & 0xFF_FFFF;
        if address >> 20 != 6 {
            return false;
        }
        let data = data as u8;
        let s = &mut self.sram;
        match (address >> 16) & 7 {
            0 => {
                if address & 2 != 0 {
                    match s[0] & 7 {
                        0 => s[6] = ((s[6] ^ s[1]) ^ data) & 0xFE,
                        1 => s[7] = data & 0xFE,
                        7 => b61 = 0x08_0000 + ((u32::from(data) & 0x1C) << 14),
                        _ => {}
                    }
                    s[1] = data;
                } else {
                    s[0] = data;
                }
            }
            1 => {
                if address & 2 != 0 {
                    s[2] = data;
                }
            }
            4 => {
                if address & 2 != 0 { s[4] = data } else { s[3] = data }
            }
            7 => {
                if address & 2 == 0 {
                    s[5] = data;
                    if s[2] & 0x80 != 0 {
                        b60 = 0x08_0000 + ((u32::from(data) & 0x1C) << 14);
                    }
                }
            }
            _ => {}
        }
        self.smw64 = Some((b60, b61));
        true
    }

    pub fn write(&mut self, address: u32, value: u8) {
        if (self.sram_bank(address) && self.sram_writable) || self.extra_sram_bank(address) {
            self.sram[(address & 0xFFFF) as usize] = value;
        }
    }

    /// A write to $A130xx ("TIME"). `rom_mask` is the padded ROM size minus one.
    pub fn time_write(&mut self, address: u32, data: u16, rom_mask: u32) {
        use crate::cart_db::TimeWrite;
        let address = address & 0xFF_FFFF;
        match self.hw.time_w {
            TimeWrite::Default => {}
            TimeWrite::Unused | TimeWrite::Seganet => return,
            TimeWrite::Regs => return self.hw.default_write(address, data),
        }
        if address < 0xA1_3060 {
            // Multi-game cartridge mapper (reference default_time_w).
            self.map_linear(address);
            self.hw.bankshift = true;
        } else if address > 0xA1_30F1 {
            // Eight 512 KiB banks; the register index selects which.
            let first = ((address << 2) & 0x38) as usize;
            let page = (u32::from(data) << 19) & rom_mask;
            for i in 0..8 {
                self.banks[first + i] = page + ((i as u32) << 16);
            }
        } else if data & 1 != 0 {
            if self.has_sram() {
                if !self.sram_mapped {
                    self.saved_bank = self.banks[0x20];
                    self.sram_mapped = true;
                }
                self.sram_writable = data & 2 == 0;
            }
        } else if self.sram_mapped {
            self.banks[0x20] = self.saved_bank;
            self.sram_mapped = false;
            self.sram_writable = false;
        }
    }

    /// 64 consecutive 64 KiB banks starting at `first` (mapper_64k_multi_w).
    fn map_linear(&mut self, first: u32) {
        self.bank32 = None;
        for (i, bank) in self.banks.iter_mut().enumerate() {
            *bank = ((first + i as u32) & 0x3F) << 16;
        }
    }

    /// !TIME read through the board's handler: the 16-bit handler value, or
    /// `None` for open bus. `bus` is the open-bus word at the 68000's PC.
    pub fn time_read(&mut self, address: u32, bus: u16) -> Option<u16> {
        use crate::cart_db::TimeRead;
        let address = address & 0xFF_FFFF;
        let bus8 = u16::from(bus.to_be_bytes()[(address & 1) as usize]);
        match self.hw.time_r {
            TimeRead::None => None,
            TimeRead::Regs => Some(self.hw.matching(address).map_or(bus8, |i| u16::from(self.hw.regs[i]))),
            TimeRead::CustomRegs => Some(self.hw.matching(address).map_or(bus8, |i| u16::from(self.hw.regs[i] >> 1))),
            TimeRead::Multi128k => {
                let bank = ((address & 0x02) << 2) | (address & 0x04) | ((address & 0x10) >> 3) | ((address & 0x20) >> 5);
                self.map_linear(bank << 1);
                Some(0x03)
            }
            TimeRead::Radica64k => {
                let index = (address >> 1) & 0x3F;
                self.bank32 = None;
                for (i, bank) in self.banks.iter_mut().enumerate() {
                    *bank = (index | i as u32) << 16;
                }
                Some(0xFFFF)
            }
            TimeRead::Sf004 => Some(((self.banks[0] >> 18) << 4) as u16),
            TimeRead::Radica128k => {
                let index = (address >> 1) & 0x3E;
                self.bank32 = None;
                for (i, bank) in self.banks.iter_mut().enumerate().take(0x20) {
                    *bank = (index | i as u32) << 16;
                }
                // $200000-$3FFFFF: 8 KiB backup RAM, mirrored.
                if self.sram.is_empty() {
                    self.sram = vec![0xFF; SRAM_SIZE];
                }
                self.extra_sram_banks = (0x20, 0x40);
                Some(0xFFFF)
            }
        }
    }

    /// Register-bank read ($400000-$7FFFFF, or a database bank in the cart
    /// window). Returns the handler value (byte reads use its low byte).
    pub fn regs_read(&self, address: u32, bus: u16, pad: u8) -> Option<u16> {
        use crate::cart_db::RegsRead;
        let address = address & 0xFF_FFFF;
        if !self.hw.regs_bank(address, false) {
            return None;
        }
        let bus8 = u16::from(bus.to_be_bytes()[(address & 1) as usize]);
        Some(match self.hw.regs_r {
            RegsRead::None => return None,
            RegsRead::Default => self.hw.matching(address).map_or(bus8, |i| u16::from(self.hw.regs[i])),
            RegsRead::Default16 => self.hw.matching(address).map_or(bus, |i| u16::from(self.hw.regs[i]) << 8),
            RegsRead::Tekken => {
                if address & 0x0E == 0x02 {
                    u16::from(self.hw.regs[0]).wrapping_sub(1)
                } else {
                    bus
                }
            }
            RegsRead::TopShooter => {
                if address < 0x20_2000 {
                    let pressed = |b: crate::input::Button| pad & b.mask() != 0;
                    use crate::input::Button as B;
                    let mut temp = 0xFFu8;
                    match address & 0xFF {
                        0x43 => {
                            if pressed(B::A) { temp &= !0x80; }
                            if pressed(B::B) { temp &= !0x10; }
                            if pressed(B::Start) { temp &= !0x20; }
                        }
                        0x45 => {
                            if pressed(B::Up) { temp &= !0x08; }
                            if pressed(B::Down) { temp &= !0x10; }
                        }
                        0x47 => {
                            if pressed(B::Right) { temp &= !0x03; }
                        }
                        0x49 => {
                            if pressed(B::Left) { temp &= !0x03; }
                            if pressed(B::C) { temp &= !0x01; }
                        }
                        0x51 => temp = 0xA5,
                        _ => temp = bus8 as u8,
                    }
                    u16::from(temp)
                } else {
                    u16::from(self.sram.get((address & 0xFFFF) as usize).copied().unwrap_or(0xFF))
                }
            }
        })
    }

    /// Register-bank write; returns whether the board took it.
    pub fn regs_write(&mut self, address: u32, data: u16) -> bool {
        use crate::cart_db::RegsWrite;
        let address = address & 0xFF_FFFF;
        if !self.hw.regs_bank(address, true) {
            return false;
        }
        match self.hw.regs_w {
            RegsWrite::None => return false,
            RegsWrite::Default => self.hw.default_write(address, data),
            RegsWrite::Custom => {
                if address >> 16 > 0x6F {
                    // mapper_32k_w: 64 x 32 KiB banks into $000000-$0FFFFF.
                    if data != 0 {
                        self.bank32 = Some(data);
                    } else {
                        self.bank32 = None;
                        for (i, bank) in self.banks.iter_mut().enumerate().take(0x10) {
                            *bank = (i as u32) << 16;
                        }
                    }
                } else {
                    self.hw.custom_write(address, data);
                }
            }
            RegsWrite::CustomAlt => {
                if address >> 16 > 0x5F {
                    // mapper_64k_w: one 64 KiB bank across $000000-$0FFFFF.
                    self.bank32 = None;
                    for (i, bank) in self.banks.iter_mut().enumerate().take(0x10) {
                        *bank = if data != 0 { u32::from(data & 0xF) << 16 } else { ((i as u32) & 0xF) << 16 };
                    }
                } else {
                    self.hw.default_write(address, data);
                }
            }
            RegsWrite::Realtec => match address {
                0x40_2000 => self.hw.regs[1] = ((data & 3) << 1) as u8,
                0x40_4000 => self.hw.regs[2] = ((data & 3) << 1) as u8,
                0x40_0000 => {
                    if data & 1 != 0 && self.hw.regs[0] == 0 {
                        let (fixed, value) = (u32::from(self.hw.regs[1]), u32::from(self.hw.regs[2]));
                        for (i, bank) in self.banks.iter_mut().enumerate() {
                            let base = ((i as u32) & 7 & !fixed) | (value & fixed);
                            *bank = base << 16;
                        }
                        self.realtec_boot = 0;
                        self.hw.regs[0] = data as u8;
                    }
                }
                _ => {}
            },
            RegsWrite::Tekken => self.hw.tekken_write(address, data),
            RegsWrite::TopShooter => {
                if address >= 0x20_2000 {
                    if self.sram.is_empty() {
                        self.sram = vec![0xFF; SRAM_SIZE];
                    }
                    self.sram[(address & 0xFFFF) as usize] = data as u8;
                }
            }
            RegsWrite::Multi256k => {
                let bank = ((u32::from(data) & 0x01) << 2) | ((u32::from(data) & 0xC0) >> 6);
                self.map_linear(bank << 2);
            }
        }
        true
    }

    /// Realtec: a word read of $000100 maps cartridge ROM into bank 0 and
    /// asserts /VRES (the 68000 resets; TMSS bypass).
    pub fn realtec_word_read(&mut self, address: u32) {
        if self.hw.realtec && address & 0xFF_FFFF == 0x100 {
            self.realtec_boot &= !1;
            self.banks[0] = 0;
            self.reset_request = true;
        }
    }

    pub fn valid(&self) -> bool {
        self.banks.len() == 64
            && (self.sram.is_empty() || self.sram.len() == SRAM_SIZE)
            && self.sram_start <= 0xFF_FFFF
    }
}

/// Console region reported in the version register ($A10001 bits 7-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleRegion {
    JapanNtsc,
    JapanPal,
    Usa,
    Europe,
}

impl ConsoleRegion {
    /// Version register value: region bits plus "no expansion unit" (bit 5).
    pub fn version_register(self) -> u8 {
        0x20 | match self {
            Self::JapanNtsc => 0x00,
            Self::JapanPal => 0x40,
            Self::Usa => 0x80,
            Self::Europe => 0xC0,
        }
    }

    /// PAL consoles: 313-line 50 Hz video and the PAL master clock.
    pub fn is_pal(self) -> bool {
        matches!(self, Self::JapanPal | Self::Europe)
    }

    /// Header country auto-detection as in the reference: USA, then Japan,
    /// then Europe, then Japan PAL; a few known mislabeled titles need PAL.
    pub fn detect(data: &[u8]) -> Self {
        let country = data.get(0x1F0..0x1F4).unwrap_or(&[]);
        let product = header_text(data, 0x180..0x18E);
        let mut bits = 0u8;
        let prefix = |text: &[u8]| country.len() >= 3 && &country[..3] == text;
        if prefix(b"eur") || prefix(b"EUR") || prefix(b"Eur") {
            bits |= 8;
        } else if prefix(b"jap") || prefix(b"JAP") {
            bits |= 1;
        } else if prefix(b"usa") || prefix(b"USA") {
            bits |= 4;
        } else {
            for &c in country {
                let c = c.to_ascii_uppercase();
                bits |= match c {
                    b'U' => 4,
                    b'J' | b'K' => 1,
                    b'E' => 8,
                    0..=15 => c,
                    // A signed char >= 0x80 is negative, passes the
                    // reference's `c < 16` test and ORs its low bits in.
                    0x80..=0xFF => c & 0x0F,
                    b'0'..=b'9' => c - b'0',
                    b'A'..=b'F' => c - b'A' + 10,
                    _ => 0,
                };
            }
        }
        // Reference overrides for headers that name the wrong region.
        let header_checksum = data.get(0x18E..0x190).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]));
        let real_checksum = data
            .get(0x200..)
            .unwrap_or(&[])
            .chunks(2)
            .fold(0u16, |sum, w| sum.wrapping_add(u16::from_be_bytes([w[0], *w.get(1).unwrap_or(&0)])));
        let needs_pal = ["T-69046-50", "T-120106-00", "T-97126 -50", "T-113026-50", "T-70096 -00"]
            .iter()
            .any(|code| product.contains(code))
            || (product.contains("T-45033") && header_checksum == 0x0F81) // Alisia Dragoon (Europe)
            || (header_checksum == 0 && real_checksum == 0x1F7F); // Radica Sensible Soccer Plus
        if needs_pal {
            Self::Europe
        } else if real_checksum == 0x532E && product.contains("1011-00") {
            Self::JapanNtsc // On Dal Jang Goon (Korea)
        } else if bits & 4 != 0 {
            Self::Usa
        } else if bits & 1 != 0 {
            Self::JapanNtsc
        } else if bits & 8 != 0 {
            Self::Europe
        } else if bits & 2 != 0 {
            Self::JapanPal
        } else {
            Self::Usa
        }
    }
}

#[cfg(test)]
mod region_tests {
    use super::ConsoleRegion;

    fn header(country: &[u8]) -> Vec<u8> {
        let mut data = vec![b' '; 0x200];
        data[0x1F0..0x1F0 + country.len()].copy_from_slice(country);
        data
    }

    #[test]
    fn header_country_codes_select_the_reference_default_console() {
        for (country, region) in [
            (&b"JUE"[..], ConsoleRegion::Usa),
            (b"J  ", ConsoleRegion::JapanNtsc),
            (b"E  ", ConsoleRegion::Europe),
            (b"JE ", ConsoleRegion::JapanNtsc),
            (b"8  ", ConsoleRegion::Europe),
            (b"4  ", ConsoleRegion::Usa),
            (b"1  ", ConsoleRegion::JapanNtsc),
            (b"F  ", ConsoleRegion::Usa),
            (b"EUROPE", ConsoleRegion::Europe),
            (b"   ", ConsoleRegion::Usa),
        ] {
            assert_eq!(ConsoleRegion::detect(&header(country)), region, "{country:?}");
        }
        assert_eq!(ConsoleRegion::Usa.version_register(), 0xA0);
        assert_eq!(ConsoleRegion::Europe.version_register(), 0xE0);
    }
}
