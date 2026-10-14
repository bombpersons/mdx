//! Unlicensed and special cartridge hardware from the reference's cartridge
//! database (GPGX md_cart.c `rom_database`, matched by header and computed
//! checksum): protection registers in $400000-$7FFFFF or the !TIME range
//! ($A130xx), and the bank mappers those boards use. The table is generated
//! from the reference source; handler semantics follow it line by line.
use serde::{Deserialize, Serialize};

/// Read handler for the database's register banks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RegsRead {
    #[default]
    None,
    Default,
    Default16,
    Tekken,
    TopShooter,
}

/// Write handler for the database's register banks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RegsWrite {
    #[default]
    None,
    Default,
    Custom,
    CustomAlt,
    Realtec,
    Tekken,
    TopShooter,
    Multi256k,
}

/// !TIME ($A130xx) read handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TimeRead {
    #[default]
    None,
    Regs,
    CustomRegs,
    Multi128k,
    Radica64k,
    Radica128k,
    /// SF-004: index of the first-page 256 KiB bank.
    Sf004,
}

/// !TIME ($A130xx) write handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TimeWrite {
    /// Multi-game 64 KiB mapper below $A13060, 512 KiB pages above $A130F1,
    /// the SRAM switch in between (reference default_time_w).
    #[default]
    Default,
    Unused,
    Regs,
    Seganet,
}

#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub chk1: u16,
    pub chk2: u16,
    pub banks: (u8, u8),
    pub regs: [u8; 4],
    pub mask: [u32; 4],
    pub addr: [u32; 4],
    pub realtec: bool,
    pub bankshift: bool,
    pub time_r: TimeRead,
    pub time_w: TimeWrite,
    pub regs_r: RegsRead,
    pub regs_w: RegsWrite,
}

/// Cartridge hardware registers and handlers (reference cart_hw_t).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Hardware {
    pub regs: [u8; 4],
    pub mask: [u32; 4],
    pub addr: [u32; 4],
    /// Inclusive 64 KiB bank range using `regs_r`/`regs_w`.
    pub banks: (u8, u8),
    pub realtec: bool,
    pub bankshift: bool,
    pub time_r: TimeRead,
    pub time_w: TimeWrite,
    pub regs_r: RegsRead,
    pub regs_w: RegsWrite,
}

impl Hardware {
    pub fn from_entry(e: &Entry) -> Self {
        Self {
            regs: e.regs,
            mask: e.mask,
            addr: e.addr,
            banks: e.banks,
            realtec: e.realtec,
            bankshift: e.bankshift,
            time_r: e.time_r,
            time_w: e.time_w,
            regs_r: e.regs_r,
            regs_w: e.regs_w,
        }
    }

    /// Whether `address` falls in a bank served by the register handlers.
    pub fn regs_bank(&self, address: u32, write: bool) -> bool {
        let bank = ((address >> 16) & 0xFF) as u8;
        let handler = if write { self.regs_w != RegsWrite::None } else { self.regs_r != RegsRead::None };
        handler && (self.banks.0..=self.banks.1).contains(&bank)
    }

    /// Index of the register matching `address`.
    pub fn matching(&self, address: u32) -> Option<usize> {
        (0..4).find(|&i| address & self.mask[i] == self.addr[i])
    }

    /// default_regs_w: store into the matching register (low byte).
    pub fn default_write(&mut self, address: u32, data: u16) {
        if let Some(i) = self.matching(address) {
            self.regs[i] = data as u8;
        }
    }

    /// custom_regs_w after the bank check: store, then the bit-swapping.
    pub fn custom_write(&mut self, address: u32, data: u16) {
        self.default_write(address, data);
        let temp = self.regs[0];
        self.regs[2] = match self.regs[1] & 3 {
            0 => temp << 1,
            1 => temp >> 1,
            2 => (temp >> 4) | ((temp & 0x0F) << 4),
            _ => temp.reverse_bits(),
        };
    }

    /// tekken_regs_w.
    pub fn tekken_write(&mut self, address: u32, data: u16) {
        match address & 0x0E {
            0x00 => self.regs[0] = 0,
            0x02 | 0x0C | 0x0E => {}
            _ => {
                if data & 1 != 0 {
                    self.regs[0] |= 1 << (((address.wrapping_sub(0x04)) >> 1) & 3);
                }
            }
        }
    }
}

/// Super Fighter Team boards (header ROM type `SF`, product 001/002/004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SfBoard {
    #[default]
    None,
    Sf001,
    Sf002,
    Sf004,
}

/// Reference database lookup by header checksum and computed checksum.
pub fn lookup(header_checksum: u16, real_checksum: u16) -> Option<&'static Entry> {
    DATABASE.iter().find(|e| e.chk1 == header_checksum && e.chk2 == real_checksum)
}

pub static DATABASE: &[Entry] = &[
    // Funny World & Balloon Boy
    Entry { chk1: 0x0000, chk2: 0x06ab, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: true, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Realtec },
    // Whac-a-Critter - Mallet Legend
    Entry { chk1: 0xffff, chk2: 0xf863, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: true, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Realtec },
    // Earth Defense
    Entry { chk1: 0xffff, chk2: 0x44fb, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: true, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Realtec },
    // Tom Clown
    Entry { chk1: 0x0000, chk2: 0xc0cd, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: true, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Realtec },
    // 1800-in-1
    Entry { chk1: 0x3296, chk2: 0x2370, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::Multi128k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Golden Mega 250-in-1
    Entry { chk1: 0xe43c, chk2: 0x886f, banks: (0x08, 0x08), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Multi256k },
    // RADICA (Volume 1) (bad dump ?)
    Entry { chk1: 0x0000, chk2: 0x2326, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::Radica64k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // RADICA (Volume 1)
    Entry { chk1: 0x24f4, chk2: 0xfc84, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Radica64k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // RADICA (Volume 2)
    Entry { chk1: 0xd951, chk2: 0x78d0, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Radica64k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // RADICA (Volume 3 - Super Sonic Gold edition)
    Entry { chk1: 0x0000, chk2: 0x1f25, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Radica64k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // RADICA (Street Fighter II CE edition)
    Entry { chk1: 0x1add, chk2: 0xa838, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Radica64k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // RADICA (Street Fighter II CE edition) (PAL)
    Entry { chk1: 0x104f, chk2: 0x32e9, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Radica64k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // RADICA (Sensible Soccer Plus edition) (PAL)
    Entry { chk1: 0x0000, chk2: 0x1f7f, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Radica128k, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Tenchi wo Kurau III: Sangokushi Gaiden - Chinese Fighter
    Entry { chk1: 0x9490, chk2: 0x8180, banks: (0x40, 0x6f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf0000c, 0xf0000c, 0xf0000c, 0xf0000c], addr: [0x400000, 0x400004, 0x400008, 0x40000c], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::CustomAlt },
    // Top Fighter
    Entry { chk1: 0x4eb9, chk2: 0x5d8b, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Soul Edge VS Samurai Spirits
    Entry { chk1: 0x00ff, chk2: 0x5d34, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Mulan
    Entry { chk1: 0x0404, chk2: 0x1b40, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Pocket Monsters II
    Entry { chk1: 0x47f9, chk2: 0x17e5, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Lion King 3
    Entry { chk1: 0x0000, chk2: 0x507c, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Super King Kong 99
    Entry { chk1: 0x0000, chk2: 0x7d6e, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Super Donkey Kong 99 (bootleg version of Super King Kong 99 ? all protection checks seem patched except one, resulting in broken end game sequence if hardware is not implemented)
    Entry { chk1: 0x0000, chk2: 0x16dc, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Gunfight 3-in-1
    Entry { chk1: 0x0000, chk2: 0x6ff8, banks: (0x60, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xf00007, 0xf00007, 0xf00007, 0xffffff], addr: [0x600001, 0x600003, 0x600005, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Custom },
    // Pokemon Stadium
    Entry { chk1: 0x0000, chk2: 0x843c, banks: (0x70, 0x7f), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: true, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Custom },
    // Tekken 3 Special (original dump) (a bootleg version also exists, with patched protection & different boot routine which reads unused !TIME mapped area)
    Entry { chk1: 0x0000, chk2: 0xc2f0, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Tekken, regs_w: RegsWrite::Tekken },
    // Lion King 2
    Entry { chk1: 0xffff, chk2: 0x1d9b, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xfffffd, 0xfffffd, 0xffffff, 0xffffff], addr: [0x400000, 0x400004, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Default },
    // Squirell King
    Entry { chk1: 0x0000, chk2: 0x8ec8, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xfffffd, 0xfffffd, 0xffffff, 0xffffff], addr: [0x400000, 0x400004, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Default },
    // Tiny Toon Adventures 3
    Entry { chk1: 0x2020, chk2: 0xed9c, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xfffffd, 0xfffffd, 0xffffff, 0xffffff], addr: [0x400000, 0x400004, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Default },
    // Barver Battle Saga - Tai Kong Zhan Shi (registers accessed by Z80, related to sound engine ?)
    Entry { chk1: 0x30b9, chk2: 0x1c2a, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xfffffd, 0xfffffd, 0xffffff, 0xffffff], addr: [0x400000, 0x400004, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Default },
    // Shui Hu Zhuan (registers accessed by Z80, related to sound engine ?)
    Entry { chk1: 0x6001, chk2: 0x0211, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xfffffd, 0xfffffd, 0xffffff, 0xffffff], addr: [0x400000, 0x400004, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Default },
    // Feng Shen Ying Jie Chuan (registers accessed by Z80, related to sound engine ?)
    Entry { chk1: 0xffff, chk2: 0x5d98, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xfffffd, 0xfffffd, 0xffffff, 0xffffff], addr: [0x400000, 0x400004, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Default },
    // (*) Shui Hu: Feng Yun Zhuan (patched ROM, unused registers)
    Entry { chk1: 0x3332, chk2: 0x872b, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xfffffd, 0xfffffd, 0xffffff, 0xffffff], addr: [0x400000, 0x400004, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::Default },
    // (*) Chao Ji Da Fu Weng (patched ROM, various words witten to register, long word also read from $7E0000, unknown banking hardware ?)
    Entry { chk1: 0xa697, chk2: 0xa697, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Default },
    // (*) Aq Renkan Awa - A Q Lian Huan Pao (patched ROM, ON/OFF bit sequence is written to register, unknown banking hardware ?)
    Entry { chk1: 0x8104, chk2: 0x0517, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400001, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::Default },
    // (*) Tun Shi Tian Di III (patched ROM, unused register)
    Entry { chk1: 0x0000, chk2: 0x9c5e, banks: (0x40, 0x40), regs: [0xab, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400046, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Chao Ji Mahjong Club (uses 16-bits reads)
    Entry { chk1: 0x0000, chk2: 0x2d5c, banks: (0x40, 0x40), regs: [0x90, 0xd3, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default16, regs_w: RegsWrite::None },
    // Ma Jiang Qing Ren: Ji Ma Jiang Zhi
    Entry { chk1: 0x0000, chk2: 0x7037, banks: (0x40, 0x40), regs: [0x90, 0xd3, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x401000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Du Shen Zhi Meng Huan Poker
    Entry { chk1: 0x0000, chk2: 0x3b95, banks: (0x40, 0x40), regs: [0x90, 0xd3, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x401000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Feng Kuang Tao Hua Yuan (original version from Creaton Softec Inc) (a bootleg version also exists with patched protection and minor title screen variations)
    Entry { chk1: 0x0000, chk2: 0x9dc4, banks: (0x40, 0x40), regs: [0x90, 0xd3, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x401000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // (*) Zhan Qi - Chinese Battle Chess (patched ROM ? registers values are read and compared to expected values but comparison results are unused)
    Entry { chk1: 0x0000, chk2: 0x4704, banks: (0x40, 0x40), regs: [0x90, 0xd3, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x401000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // (*) Jiu Ji Ma Jiang II: Ye Yan Bian (patched ROM, using expected register value $f0 crashes the game) (uses 16-bits reads)
    Entry { chk1: 0x0c44, chk2: 0xba81, banks: (0x40, 0x40), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default16, regs_w: RegsWrite::None },
    // Shi Liu Zhang Ma Jiang - 16 Tiles Mahjong (uses 16-bits reads)
    Entry { chk1: 0xfb40, chk2: 0x4bed, banks: (0x40, 0x40), regs: [0x00, 0xaa, 0x00, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x400002, 0x000000, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default16, regs_w: RegsWrite::None },
    // Shi Liu Zhang Ma Jiang II - 16 Tiles Mahjong II (uses 16-bits reads)
    Entry { chk1: 0xffff, chk2: 0x0903, banks: (0x40, 0x40), regs: [0x00, 0x00, 0xc9, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x400004, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default16, regs_w: RegsWrite::None },
    // Lei Dian Chuan Shuo II - Thunderbolt II (uses 16-bits reads)
    Entry { chk1: 0x0000, chk2: 0x1585, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xaa, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default16, regs_w: RegsWrite::None },
    // Chao Ji Poker (correct ROM dump, original release is an overdump)
    Entry { chk1: 0xffff, chk2: 0xd7b0, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xaa, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Super Bubble Bobble
    Entry { chk1: 0x0000, chk2: 0x16cd, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // San Guo Yan Yi: Huo Shao Chi Bi - The Battle of Red Cliffs
    Entry { chk1: 0x0000, chk2: 0xed61, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xaa, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Huan Le Tao Qi Shu - Smart Mouse
    Entry { chk1: 0x0000, chk2: 0x1a28, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xaa, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Sichuan Style Mahjong
    Entry { chk1: 0x0000, chk2: 0x71e1, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xaa, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // (*) Hei Tao 2 - Super Big 2 (patched ROM, unused registers)
    Entry { chk1: 0x0000, chk2: 0x5843, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xaa, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Mighty Morphin Power Rangers - The Fighting Edition
    Entry { chk1: 0x0000, chk2: 0x2288, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xc9, 0x18], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Ling Huan Dao Shi - Super Magican
    Entry { chk1: 0x0080, chk2: 0x3dba, banks: (0x40, 0x40), regs: [0x55, 0x0f, 0xc9, 0x18], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Ya-Se Chuanshuo
    Entry { chk1: 0xffff, chk2: 0xd472, banks: (0x40, 0x40), regs: [0x63, 0x98, 0xc9, 0x18], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Meng Huan Shui Guo Pan - 777 Casino (For first one, 0x55 works as well. Other values seem never used so they are guessed from other unlicensed games using similar mapper)
    Entry { chk1: 0x0000, chk2: 0xf8d9, banks: (0x40, 0x40), regs: [0x63, 0x98, 0xc9, 0x18], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Wu Kong Wai Zhuan (original) (a bootleg version also exists, with patched protection & modified SRAM test routine ?)
    Entry { chk1: 0x0000, chk2: 0x19ff, banks: (0x40, 0x40), regs: [0x63, 0x98, 0xc9, 0x18], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Soul Blade
    Entry { chk1: 0x0000, chk2: 0x0c5b, banks: (0x40, 0x40), regs: [0x63, 0x98, 0xc9, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Shi San Zhang Ma Jiang: Zhong Guo Mei Nv Pian (only $400004 seem checked, for which 0xaa works as well, other addresses/values are guessed from other unlicensed games using similar mapper)
    Entry { chk1: 0xffff, chk2: 0xd41d, banks: (0x40, 0x40), regs: [0x63, 0x98, 0xc9, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Rockman X3 (original version) (only two last addresses seem checked, for which 0xaa/0x18 work as well, other addresses/values are guessed from other unlicensed games using similar mapper)
    Entry { chk1: 0xffff, chk2: 0x6fdd, banks: (0x40, 0x40), regs: [0x63, 0x98, 0xc9, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x400000, 0x400002, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // King of Fighter 98
    Entry { chk1: 0x0000, chk2: 0xd0a0, banks: (0x48, 0x4f), regs: [0x00, 0x00, 0xaa, 0xf0], mask: [0xffffff, 0xffffff, 0xfc0000, 0xfc0000], addr: [0x000000, 0x000000, 0x480000, 0x4c0000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Rock Heaven
    Entry { chk1: 0x6cca, chk2: 0x2395, banks: (0x50, 0x50), regs: [0x50, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x500008, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Rock World
    Entry { chk1: 0x3547, chk2: 0xa3da, banks: (0x50, 0x50), regs: [0x50, 0xa0, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x500008, 0x500208, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // Rockman X3 (bootleg version ? two last register returned values are ignored, note that 0xaa/0x18 would work as well)
    Entry { chk1: 0x0000, chk2: 0x9d0e, banks: (0x40, 0x40), regs: [0x0c, 0x00, 0xc9, 0xf0], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0x000000, 0x400004, 0x400006], realtec: false, bankshift: false, time_r: TimeRead::Regs, time_w: TimeWrite::Unused, regs_r: RegsRead::Default, regs_w: RegsWrite::None },
    // (*) Dragon Ball Final Bout  (patched ROM, in original code, different switches occurs depending on returned value $00-$0f)
    Entry { chk1: 0xc65a, chk2: 0xc65a, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Regs, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // (*) Yi Men Ying Lie: Yang Jia Jiang - Yang Warrior Family (patched ROM, register value unused)
    Entry { chk1: 0x0000, chk2: 0x96b0, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Regs, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Super Mario 2 1998
    Entry { chk1: 0xffff, chk2: 0x0474, banks: (0x00, 0x00), regs: [0x0a, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Regs, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Super Mario World
    Entry { chk1: 0x2020, chk2: 0xb4eb, banks: (0x00, 0x00), regs: [0x1c, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Regs, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // King of Fighter 99
    Entry { chk1: 0x0000, chk2: 0x021e, banks: (0x00, 0x00), regs: [0x00, 0x01, 0x1f, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0xa13002, 0xa1303e, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::CustomRegs, time_w: TimeWrite::Regs, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Pocket Monster
    Entry { chk1: 0xd6fc, chk2: 0x1eb1, banks: (0x00, 0x00), regs: [0x00, 0x01, 0x1f, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0xa13002, 0xa1303e, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::CustomRegs, time_w: TimeWrite::Regs, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Pocket Monster (bootleg version ? two last register returned values are ignored & first register test has been modified)
    Entry { chk1: 0xd6fc, chk2: 0x6319, banks: (0x00, 0x00), regs: [0x14, 0x01, 0x1f, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0xa13002, 0xa1303e, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Regs, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // A Bug's Life (bootleg version ? two last register returned values are ignored & first register test has been modified ?)
    Entry { chk1: 0x7f7f, chk2: 0x2aad, banks: (0x00, 0x00), regs: [0x28, 0x01, 0x1f, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0xa13000, 0xa13002, 0xa1303e, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::Regs, time_w: TimeWrite::Unused, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Game no Kanzume Otokuyou
    Entry { chk1: 0x0000, chk2: 0xf9d1, banks: (0x00, 0x00), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Seganet, regs_r: RegsRead::None, regs_w: RegsWrite::None },
    // Top Shooter (arcade hardware)
    Entry { chk1: 0xffff, chk2: 0x3632, banks: (0x20, 0x20), regs: [0x00, 0x00, 0x00, 0x00], mask: [0xffffff, 0xffffff, 0xffffff, 0xffffff], addr: [0x000000, 0x000000, 0x000000, 0x000000], realtec: false, bankshift: false, time_r: TimeRead::None, time_w: TimeWrite::Unused, regs_r: RegsRead::TopShooter, regs_w: RegsWrite::TopShooter },
];
