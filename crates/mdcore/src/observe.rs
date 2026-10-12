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
