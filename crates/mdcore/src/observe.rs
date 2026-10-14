//! Opt-in work-RAM bus-write observations. Matching uses folded offsets,
//! includes same-value writes, and retains counts when the event cap is reached.
use serde::Serialize;

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct UnmappedAccess {
    pub address: u32,
    pub width_bits: u8,
    pub write: bool,
    pub pc: Option<u32>,
    pub writer: String,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct WramWrite {
    pub pc: Option<u32>,
    pub address: u32,
    pub folded_offset: usize,
    pub width_bits: u8,
    pub before: u32,
    pub value: u32,
    pub frame: u64,
    pub writer: String,
    /// 68000 pre-step registers; unavailable for Z80/external bus writes.
    pub d: Option<[u32; 8]>,
    pub a: Option<[u32; 8]>,
    pub sr: Option<u16>,
}

pub struct WramWatch {
    pub offset: usize,
    pub length: usize,
    pub max_records: usize,
    pub writing_pc: Option<u32>,
    pub observed: u64,
    pub records: Vec<WramWrite>,
}

impl WramWatch {
    pub fn new(
        offset: usize,
        length: usize,
        max_records: usize,
        writing_pc: Option<u32>,
    ) -> Option<Self> {
        if offset >= 65536 || length == 0 || length > 65536 || max_records > 100_000 {
            return None;
        }
        Some(Self {
            offset,
            length,
            max_records,
            writing_pc,
            observed: 0,
            records: Vec::new(),
        })
    }
    pub fn matches(&self, address: u32, width_bits: u8, pc: Option<u32>) -> bool {
        if self.writing_pc.is_some_and(|filter| Some(filter) != pc) {
            return false;
        }
        (0..u32::from(width_bits) / 8).any(|i| {
            let offset = crate::memmap::fold_to_wram(address.wrapping_add(i)) as usize;
            (offset + 65536 - self.offset) % 65536 < self.length
        })
    }
}

/// Which sound-side register or memory a 68000 write reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SoundCommandTarget {
    /// Z80 sound RAM, 0xA00000..0xA03FFF (8 KiB mirrored).
    Z80Ram,
    /// Z80 bank register, one bit per write, 0xA06000.
    BankRegister,
    /// Z80 BUSREQ, 0xA11100.
    BusRequest,
    /// Z80 RESET, 0xA11200.
    Reset,
}

/// One 68000 write into the sound side of the console.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SoundCommand {
    /// Completed video frames when the write happened (absolute).
    pub frame: u64,
    pub master_cycle: u64,
    /// The writing 68000 instruction.
    pub pc: u32,
    pub target: SoundCommandTarget,
    /// 68000 bus address as written.
    pub address: u32,
    /// Offset within Z80 RAM, for `z80_ram` writes.
    pub z80_address: Option<u16>,
    pub value: u8,
    /// Whether the 68000 held the Z80 bus. Z80 RAM and bank writes without
    /// it are dropped by the console but still recorded here.
    pub z80_bus_granted: bool,
}

/// Opt-in, bounded record of 68000 sound-side writes. Counts keep growing
/// after `max_records` is reached so omissions are explicit.
pub struct SoundCommandLog {
    pub max_records: usize,
    /// Optional Z80 RAM window (offset, length); other targets are always kept.
    pub z80_range: Option<(u16, u16)>,
    /// Record BUSREQ/RESET writes; otherwise they are only counted.
    pub bus_control: bool,
    pub observed: u64,
    /// BUSREQ/RESET writes counted but not recorded.
    pub bus_control_counted: u64,
    pub records: Vec<SoundCommand>,
}

impl SoundCommandLog {
    pub fn new(max_records: usize, z80_range: Option<(u16, u16)>, bus_control: bool) -> Self {
        Self { max_records, z80_range, bus_control, observed: 0, bus_control_counted: 0, records: Vec::new() }
    }

    pub fn wants(&self, target: SoundCommandTarget, z80_address: Option<u16>) -> bool {
        match (target, z80_address, self.z80_range) {
            (SoundCommandTarget::Z80Ram, Some(at), Some((start, len))) => {
                at >= start && u32::from(at) < u32::from(start) + u32::from(len)
            }
            _ => true,
        }
    }

    pub fn push(&mut self, command: SoundCommand) {
        if !self.bus_control
            && matches!(command.target, SoundCommandTarget::BusRequest | SoundCommandTarget::Reset)
        {
            self.bus_control_counted += 1;
            return;
        }
        self.observed += 1;
        if self.records.len() < self.max_records {
            self.records.push(command);
        }
    }
}

/// One write into Z80 RAM by either CPU.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Z80RamWrite {
    pub frame: u64,
    pub master_cycle: u64,
    pub z80_address: u16,
    pub before: u8,
    pub value: u8,
    /// "z80" or "m68000".
    pub writer: &'static str,
    /// The writing CPU's instruction address (Z80 or 68000 bus).
    pub pc: u32,
}

/// Opt-in watch of a Z80 RAM range; counts continue past `max_records`.
pub struct Z80RamWatch {
    pub offset: u16,
    pub length: u16,
    pub writing_pc: Option<u32>,
    pub max_records: usize,
    pub observed: u64,
    pub records: Vec<Z80RamWrite>,
}

impl Z80RamWatch {
    pub fn matches(&self, offset: u16, pc: u32) -> bool {
        offset >= self.offset
            && u32::from(offset) < u32::from(self.offset) + u32::from(self.length)
            && self.writing_pc.is_none_or(|wanted| wanted == pc)
    }
}

/// One executed Z80 instruction, with registers before it ran.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Z80TraceRecord {
    pub frame: u64,
    pub master_cycle: u64,
    pub pc: u16,
    /// Up to four instruction bytes from Z80 RAM at `pc`.
    pub bytes: [u8; 4],
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
}

/// Opt-in Z80 instruction trace, optionally limited to PCs in [start, end).
/// HALT idling is not recorded.
pub struct Z80Trace {
    pub pc_range: Option<(u16, u16)>,
    pub max_records: usize,
    pub observed: u64,
    pub records: Vec<Z80TraceRecord>,
}
