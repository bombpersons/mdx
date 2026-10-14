//! Cartridge loading and Mega Drive ROM header decoding.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Offset of the Mega Drive ROM header from the start of the ROM image.
pub const HEADER_OFFSET: usize = 0x100;
/// The full header, including the serial number and extended fields, runs from
/// 0x100 to 0x1FF.
pub const HEADER_LEN: usize = 0x100;

/// Sentinel values that appear where a field was never filled in.
const UNSET_WORDS: [u32; 2] = [0x2020_2020, 0xFFFF_FFFF];

/// Decoded 128-byte Mega Drive ROM header.
///
/// Every field keeps the raw bytes it came from, so a consumer never has to
/// trust our interpretation of a field we consider malformed.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct SegaHeader {
    pub console: String,
    pub copyright: String,
    pub domestic_name: String,
    pub overseas_name: String,
    pub serial_number: String,

    /// 32-bit big-endian ranges as declared, or `None` when the field is unset.
    pub rom_start: Option<u32>,
    pub rom_end: Option<u32>,
    pub sram_start: Option<u32>,
    pub sram_end: Option<u32>,

    pub region: String,
    pub notes: String,

    /// Raw hex for the whole header, so nothing is ever lost to parsing.
    pub raw_hex: String,

    /// Fields whose contents we treat as "never filled in by the developer".
    pub unset_fields: Vec<String>,
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn trim_nul(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    ascii(&bytes[..end]).trim().to_string()
}

/// Read a 32-bit big-endian long.
fn be32(h: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([h[off], h[off + 1], h[off + 2], h[off + 3]])
}

impl SegaHeader {
    pub fn parse(rom: &[u8]) -> Result<Self, HeaderError> {
        if rom.len() < HEADER_OFFSET + HEADER_LEN {
            return Err(HeaderError::TooSmall {
                len: rom.len(),
                needed: HEADER_OFFSET + HEADER_LEN,
            });
        }
        let h = &rom[HEADER_OFFSET..HEADER_OFFSET + HEADER_LEN];

        let mut unset = Vec::new();
        // The ranges are 32-bit big-endian longs, not 24-bit fields.
        let mut range_field = |name: &'static str, off: usize| -> Option<u32> {
            let raw = be32(h, off);
            if UNSET_WORDS.contains(&raw) {
                unset.push(name.to_string());
                None
            } else {
                Some(raw)
            }
        };

        Ok(Self {
            console: ascii(&h[0x00..0x10]),
            copyright: ascii(&h[0x10..0x20]),
            domestic_name: trim_nul(&h[0x20..0x50]),
            overseas_name: trim_nul(&h[0x50..0x80]),
            serial_number: trim_nul(&rom[0x180..0x18E]),

            rom_start: range_field("rom_start", 0xA0),
            rom_end: range_field("rom_end", 0xA4),
            sram_start: range_field("sram_start", 0xA8),
            sram_end: range_field("sram_end", 0xAC),

            region: ascii(&h[0xD0..0xE0]),
            notes: trim_nul(&h[0xE0..0x100]),

            raw_hex: hex(h),
            unset_fields: unset,
        })
    }

    /// Size of the cartridge in bytes implied by the declared ROM range.
    pub fn declared_rom_len(&self) -> Option<usize> {
        match (self.rom_start, self.rom_end) {
            (Some(s), Some(e)) if e >= s => Some((e - s + 1) as usize),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum HeaderError {
    TooSmall { len: usize, needed: usize },
}

impl std::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeaderError::TooSmall { len, needed } => {
                write!(
                    f,
                    "rom is {len} bytes, too small to contain a header (need {needed})"
                )
            }
        }
    }
}

impl std::error::Error for HeaderError {}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// One entry of the 68000 exception vector table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Vector {
    pub index: usize,
    pub name: &'static str,
    pub address: u32,
    pub address_hex: String,
    /// Where the address resolves, in cartridge terms, when it is not zero.
    pub resolves_to: Option<&'static str>,
}

/// Named vectors 0..16 of the 68000 exception table.
pub const NAMED_VECTORS: [&str; 16] = [
    "reset_sp",
    "reset_pc",
    "bus_error",
    "address_error",
    "illegal_instruction",
    "divide_by_zero",
    "chk",
    "trapv",
    "privilege_violation",
    "trace",
    "line_a",
    "line_f",
    "uninitialized_interrupt",
    "vector_12",
    "vector_14",
    "trap_15",
];

/// Decode the exception vector table.
///
/// On a Mega Drive the table at ROM offset 0 holds *pairs* of longs: an
/// initial stack pointer followed by a program counter. We therefore read each
/// 32-bit word and classify it rather than assuming a flat layout.
pub fn vectors(rom: &[u8], word_to_region: impl Fn(u32) -> Option<&'static str>) -> Vec<Vector> {
    let mut out = Vec::new();
    for i in 0..16 {
        let off = i * 4;
        if off + 4 > rom.len() {
            break;
        }
        let address = u32::from_be_bytes([rom[off], rom[off + 1], rom[off + 2], rom[off + 3]]);
        out.push(Vector {
            index: i,
            name: NAMED_VECTORS[i],
            address,
            address_hex: format!("0x{address:08X}"),
            resolves_to: if address == 0 {
                None
            } else {
                word_to_region(address)
            },
        });
    }
    out
}

/// A loaded ROM image with its identity.
#[derive(Debug, Clone)]
pub struct Cartridge {
    pub path: PathBuf,
    pub data: Vec<u8>,
    pub sha256: String,
    pub header: SegaHeader,
    /// Mirror period of the cartridge window (see `mirror_period`).
    period: usize,
}

impl Cartridge {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CartridgeError> {
        let path = path.as_ref().to_path_buf();
        let data = fs::read(&path).map_err(|source| CartridgeError::Read {
            path: path.clone(),
            source,
        })?;
        Self::from_bytes(path, data)
    }

    /// Construct an identified cartridge from owned bytes. The source label
    /// need not exist on disk, making callers and tests independent of files.
    pub fn from_bytes(path: impl AsRef<Path>, data: Vec<u8>) -> Result<Self, CartridgeError> {
        let path = path.as_ref().to_path_buf();
        let sha256 = format!("{:x}", Sha256::digest(&data));
        let header = SegaHeader::parse(&data).map_err(|source| CartridgeError::Header {
            path: path.clone(),
            source,
        })?;
        // GPGX md_cart_init: pad to the next power of two from 64 KiB with
        // FF; Sonic & Knuckles leaves $200000-$3FFFFF to a locked-on cart.
        let international = data.get(0x150..0x180).unwrap_or(&[]);
        let period = if international.windows(16).any(|w| w == b"SONIC & KNUCKLES") {
            data.len().max(0x40_0000).next_power_of_two()
        } else {
            data.len().max(0x1_0000).next_power_of_two()
        };
        Ok(Self {
            path,
            data,
            sha256,
            header,
            period,
        })
    }

    /// Default linear ROM mapping. Mirror at the modeled power-of-two period;
    /// an undumped tail is FF, rather than another copy of the file's beginning.
    /// Mapper-specific cartridge hardware is outside this mapping helper.
    /// Byte at a ROM offset (which bank mappers can place beyond 4 MiB),
    /// mirrored at the power-of-two period with an FF tail.
    pub fn offset_byte(&self, offset: u32) -> u8 {
        let offset = offset as usize % self.mirror_period();
        self.data.get(offset).copied().unwrap_or(0xff)
    }

    /// Power-of-two image period (minimum 64 KiB, 4 MiB for Sonic & Knuckles)
    /// used for mirroring; bytes past the image are FF.
    pub fn mirror_period(&self) -> usize {
        self.period
    }

    pub fn mapped_byte(&self, address: u32) -> u8 {
        let offset = crate::memmap::fold_to_cart(address) as usize % self.period;
        self.data.get(offset).copied().unwrap_or(0xff)
    }

    /// True when the image length is an exact power of two, which is the norm
    /// for Mega Drive cartridges.
    pub fn is_power_of_two(&self) -> bool {
        self.data.len().is_power_of_two()
    }
}

#[derive(Debug)]
pub enum CartridgeError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Header {
        path: PathBuf,
        source: HeaderError,
    },
}

impl std::fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CartridgeError::Read { path, source } => {
                write!(f, "cannot read {}: {source}", path.display())
            }
            CartridgeError::Header { path, source } => {
                write!(f, "{}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for CartridgeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_cartridge_header() {
        let rom = crate::test_support::cartridge().data;
        let h = SegaHeader::parse(&rom).unwrap();

        assert_eq!(h.console, "SEGA MEGA DRIVE");
        assert_eq!(h.domestic_name, "MDX SYNTHETIC CARTRIDGE");
        assert_eq!(h.copyright, "(C)MDX 2026.OCT");
        assert_eq!(h.serial_number, "GM MDXTEST -01");

        // This cart does declare its ranges, as 32-bit big-endian longs, and
        // the declared ROM length matches the file exactly.
        assert_eq!(h.rom_start, Some(0x0000_0000));
        assert_eq!(h.rom_end, Some(0x0007_FFFF));
        assert_eq!(h.declared_rom_len(), Some(0x8_0000));
        assert_eq!(h.sram_start, Some(0x00FF_0000));
        assert_eq!(h.sram_end, Some(0x00FF_FFFF));
        assert!(h.unset_fields.is_empty());
    }

    #[test]
    fn decodes_synthetic_cartridge_vectors() {
        let rom = crate::test_support::cartridge().data;
        let v = vectors(&rom, |_| None);

        // Preserve the raw vectors, including a sign-extended RAM stack alias.
        assert_eq!(v[0].address, 0xFFFF_1000, "reset SP");
        assert_eq!(v[1].address, 0x0000_0200, "reset PC");
        assert_eq!(v[2].address, 0, "bus error");
    }
}
