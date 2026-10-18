//! Per-frame work RAM samples for trajectory tests: the values a port must
//! reproduce, read at every frame boundary of a run into a CSV.
//!
//! Row N is memory when run frame N begins: the moment the frame counter
//! advances, at the start of vertical blank, before frame N's controller
//! events take effect. Row 0 is the starting state. Values are big-endian
//! (as the 68000 stores them) and decoded by type.
use crate::envelope::Failure;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Most values per run (samples x rows), to keep the CSV bounded.
const MAX_VALUES: u64 = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum SampleType {
    #[serde(rename = "u8")]
    U8,
    #[serde(rename = "s8")]
    S8,
    #[serde(rename = "u16")]
    U16,
    #[serde(rename = "s16")]
    S16,
    #[serde(rename = "u32")]
    U32,
    #[serde(rename = "s32")]
    S32,
    /// Signed 16-bit word as 8.8 fixed point.
    #[serde(rename = "fx8.8")]
    Fx8_8,
    /// Signed 32-bit longword as 16.16 fixed point.
    #[serde(rename = "fx16.16")]
    Fx16_16,
}

impl SampleType {
    fn bytes(self) -> usize {
        match self {
            Self::U8 | Self::S8 => 1,
            Self::U16 | Self::S16 | Self::Fx8_8 => 2,
            Self::U32 | Self::S32 | Self::Fx16_16 => 4,
        }
    }

    fn decode(self, bytes: &[u8]) -> String {
        let unsigned = bytes.iter().fold(0u32, |v, b| v << 8 | u32::from(*b));
        match self {
            Self::U8 | Self::U16 | Self::U32 => unsigned.to_string(),
            Self::S8 => (unsigned as u8 as i8).to_string(),
            Self::S16 => (unsigned as u16 as i16).to_string(),
            Self::S32 => (unsigned as i32).to_string(),
            Self::Fx8_8 => (f64::from(unsigned as u16 as i16) / 256.0).to_string(),
            Self::Fx16_16 => (f64::from(unsigned as i32) / 65536.0).to_string(),
        }
    }
}

/// One value to sample every frame.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleSpec {
    /// Work RAM address, hexadecimal: 0xFFD008 (or 0xD008, or any
    /// 0xE00000..0xFFFFFF alias). Multi-byte values must not cross the end.
    pub address: String,
    #[serde(rename = "type")]
    pub kind: SampleType,
    /// CSV column name: letters, digits, `_` and `.`.
    pub label: String,
}

impl SampleSpec {
    /// Parse the CLI form `ADDRESS:TYPE:LABEL`, e.g. `0xFFD008:s16:x`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts = text.splitn(3, ':');
        let (Some(address), Some(kind), Some(label)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("sample {text:?} is not ADDRESS:TYPE:LABEL"));
        };
        let kind = serde_json::from_value(serde_json::Value::String(kind.into()))
            .map_err(|_| format!("sample type {kind:?} is not one of u8 s8 u16 s16 u32 s32 fx8.8 fx16.16"))?;
        Ok(Self { address: address.into(), kind, label: label.into() })
    }
}

/// Validated samples and the rows recorded so far.
pub struct Sampler {
    columns: Vec<(usize, SampleType)>,
    csv: String,
    pub rows: usize,
    last_frame: Option<u64>,
}

impl Sampler {
    /// Checks the specs; `frames` bounds the rows.
    pub fn new(specs: &[SampleSpec], frames: Option<u64>) -> Result<Option<Self>, Failure> {
        if specs.is_empty() {
            return Ok(None);
        }
        let invalid = |message: String| crate::artifacts::failure("invalid_arguments", message);
        let frames = frames.ok_or_else(|| invalid("samples need a frames target".into()))?;
        if specs.len() > 64 {
            return Err(invalid("at most 64 samples".into()));
        }
        if specs.len() as u64 * (frames + 1) > MAX_VALUES {
            return Err(invalid(format!("samples x (frames + 1) must be at most {MAX_VALUES}")));
        }
        let mut header = vec!["run_frame".to_string(), "frame".into()];
        let mut columns = Vec::new();
        for spec in specs {
            let label_ok = !spec.label.is_empty()
                && spec.label.len() <= 64
                && spec.label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
            if !label_ok || header.contains(&spec.label) {
                return Err(invalid(format!("sample label {:?} must be unique letters, digits, _ or .", spec.label)));
            }
            let address = u32::from_str_radix(spec.address.trim_start_matches("0x").trim_start_matches("0X"), 16)
                .map_err(|_| invalid(format!("sample address {:?} is not hexadecimal", spec.address)))?;
            let offset = match address {
                0..=0xFFFF => address as usize,
                0xE0_0000..=0xFF_FFFF => (address & 0xFFFF) as usize,
                _ => return Err(invalid(format!("sample address {address:#X} is not work RAM"))),
            };
            if offset + spec.kind.bytes() > 0x10000 {
                return Err(invalid(format!("sample at {address:#X} runs past the end of work RAM")));
            }
            header.push(spec.label.clone());
            columns.push((offset, spec.kind));
        }
        Ok(Some(Self { columns, csv: header.join(",") + "\n", rows: 0, last_frame: None }))
    }

    /// Records a row when `frame` is new. Call before each step, before
    /// controller events for the frame are applied.
    pub fn observe(&mut self, frame: u64, starting_frame: u64, wram: &[u8]) {
        if self.last_frame == Some(frame) {
            return;
        }
        self.last_frame = Some(frame);
        self.rows += 1;
        self.csv += &format!("{},{frame}", frame - starting_frame);
        for &(offset, kind) in &self.columns {
            self.csv.push(',');
            self.csv += &kind.decode(&wram[offset..offset + kind.bytes()]);
        }
        self.csv.push('\n');
    }

    pub fn columns(&self) -> usize {
        self.columns.len()
    }

    pub fn into_csv(self) -> String {
        self.csv
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(address: &str, kind: SampleType, label: &str) -> SampleSpec {
        SampleSpec { address: address.into(), kind, label: label.into() }
    }

    #[test]
    fn rows_decode_big_endian_values_once_per_frame() {
        let mut wram = vec![0u8; 0x10000];
        wram[0xD008..0xD00A].copy_from_slice(&(-3i16).to_be_bytes());
        wram[0xD010..0xD014].copy_from_slice(&0x0001_8000u32.to_be_bytes());
        wram[0xD020..0xD022].copy_from_slice(&0xFF80u16.to_be_bytes());
        let specs = [
            spec("0xFFD008", SampleType::S16, "x"),
            spec("0xD010", SampleType::Fx16_16, "speed"),
            spec("0xE0D020", SampleType::Fx8_8, "dy"),
        ];
        let mut sampler = Sampler::new(&specs, Some(10)).unwrap().unwrap();
        sampler.observe(100, 100, &wram);
        sampler.observe(100, 100, &wram);
        sampler.observe(101, 100, &wram);
        assert_eq!(sampler.rows, 2);
        assert_eq!(sampler.into_csv(), "run_frame,frame,x,speed,dy\n0,100,-3,1.5,-0.5\n1,101,-3,1.5,-0.5\n");
    }

    #[test]
    fn specs_are_checked() {
        let bad = |specs: &[SampleSpec], frames| Sampler::new(specs, frames).err().unwrap().message;
        assert!(bad(&[spec("0xFFD008", SampleType::U8, "x")], None).contains("frames"));
        assert!(bad(&[spec("0x200000", SampleType::U8, "x")], Some(1)).contains("not work RAM"));
        assert!(bad(&[spec("0xFFFFFF", SampleType::U16, "x")], Some(1)).contains("past the end"));
        assert!(bad(&[spec("0xFF0000", SampleType::U8, "frame")], Some(1)).contains("unique"));
        assert!(Sampler::new(&[], None).unwrap().is_none());
        let parsed = SampleSpec::parse("0xFFD008:fx16.16:x.pos").unwrap();
        assert_eq!((parsed.kind, parsed.label.as_str()), (SampleType::Fx16_16, "x.pos"));
        assert!(SampleSpec::parse("0xFFD008:f32:x").is_err());
    }
}
