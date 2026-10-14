//! Deterministic controller replay and diagnostic memory pokes, indexed by
//! completed frames (zero-based) relative to reset or the loaded state.
use crate::envelope::Failure;
use mdcore::input::Button;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputEvent {
    pub frame: u64,
    /// 0 and 1 are the console ports; 2 and 3 are J-Cart controllers 3 and 4
    /// (ignored by cartridges without J-Cart).
    #[serde(default)]
    #[schemars(range(max = 3))]
    pub port: usize,
    pub buttons: Vec<Button>,
}

impl InputEvent {
    pub fn mask(&self) -> u8 {
        self.buttons.iter().fold(0, |mask, b| mask | b.mask())
    }
}

/// Memory a poke writes directly, outside bus timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PokeSpace {
    /// 68000 work RAM: 0x0000..0xFFFF or any 0xE00000..0xFFFFFF alias.
    Wram,
    /// Z80 sound RAM: 0x0000..0x1FFF or the 68000 view 0xA00000..0xA01FFF.
    Z80,
}

/// Diagnostic write applied at the first instruction boundary once `frame`
/// frames have completed, before controller events of that frame take effect.
/// It is an analyst intervention, not modeled hardware behaviour.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Poke {
    pub frame: u64,
    pub space: PokeSpace,
    pub address: u32,
    /// Bytes written from `address` upward; 1..256, must not cross the space end.
    #[schemars(length(min = 1, max = 256))]
    pub bytes: Vec<u8>,
}

impl Poke {
    /// Offset of the first byte within its space, or a reason it is invalid.
    fn offset(&self) -> Result<usize, String> {
        let (offset, size) = match self.space {
            PokeSpace::Wram if self.address <= 0xFFFF => (self.address, 0x10000),
            PokeSpace::Wram if (0xE0_0000..=0xFF_FFFF).contains(&self.address) => {
                (self.address & 0xFFFF, 0x10000)
            }
            PokeSpace::Z80 if self.address <= 0x1FFF => (self.address, 0x2000),
            PokeSpace::Z80 if (0xA0_0000..=0xA0_1FFF).contains(&self.address) => {
                (self.address & 0x1FFF, 0x2000)
            }
            _ => return Err(format!("poke address {:#X} is outside {:?}", self.address, self.space)),
        };
        if self.bytes.is_empty() || self.bytes.len() > 256 {
            return Err("a poke writes 1..256 bytes".into());
        }
        if offset as usize + self.bytes.len() > size {
            return Err(format!("poke at {:#X} crosses the end of {:?}", self.address, self.space));
        }
        Ok(offset as usize)
    }

    /// Parse the CLI form `FRAME:SPACE:ADDRESS=HEXBYTES`, e.g. `120:z80:0x1C0A=98`.
    pub fn parse(text: &str) -> Result<Self, Failure> {
        let invalid = || Replay::invalid(format!("poke {text:?} is not FRAME:wram|z80:ADDRESS=HEXBYTES"));
        let (location, hex) = text.split_once('=').ok_or_else(invalid)?;
        let mut parts = location.splitn(3, ':');
        let frame = parts.next().and_then(|f| f.parse().ok()).ok_or_else(invalid)?;
        let space = match parts.next() {
            Some("wram") => PokeSpace::Wram,
            Some("z80") => PokeSpace::Z80,
            _ => return Err(invalid()),
        };
        let address = parts
            .next()
            .and_then(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
            .ok_or_else(invalid)?;
        let hex = hex.trim_start_matches("0x");
        if hex.is_empty() || hex.len() % 2 != 0 {
            return Err(invalid());
        }
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<_, _>>()
            .map_err(|_| invalid())?;
        Ok(Self { frame, space, address, bytes })
    }
}

#[derive(Debug, Deserialize, Serialize, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    /// Optional annotation; ignored by controller execution, included in source hash.
    pub note: Option<String>,
    #[serde(default)]
    #[schemars(length(max = 10000))]
    pub events: Vec<InputEvent>,
    /// Diagnostic RAM writes, ordered by frame; see `Poke`.
    #[serde(default)]
    #[schemars(length(max = 1000))]
    pub pokes: Vec<Poke>,
    #[serde(skip)]
    pub source_sha256: Option<String>,
}

impl Replay {
    pub fn load(path: Option<&Path>) -> Result<Self, Failure> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let bytes = crate::artifacts::read_bounded(path, 1024 * 1024)?;
        let mut replay: Self =
            serde_json::from_slice(&bytes).map_err(|e| Self::invalid(e.to_string()))?;
        replay.validate()?;
        replay.source_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
        Ok(replay)
    }

    /// Load the replay file, then append inline pokes (a stable merge by frame).
    pub fn load_with_pokes(path: Option<&Path>, pokes: &[Poke]) -> Result<Self, Failure> {
        let mut replay = Self::load(path)?;
        if !pokes.is_empty() {
            replay.pokes.extend(pokes.iter().cloned());
            replay.pokes.sort_by_key(|poke| poke.frame);
            replay.validate()?;
        }
        Ok(replay)
    }

    /// Apply every poke due once `elapsed` frames have completed.
    pub fn apply_pokes(&self, system: &mut mdcore::System, elapsed: u64, next: &mut usize) {
        while let Some(poke) = self.pokes.get(*next).filter(|poke| poke.frame <= elapsed) {
            let offset = poke.offset().expect("validated poke");
            let memory = match poke.space {
                PokeSpace::Wram => &mut system.wram[..],
                PokeSpace::Z80 => &mut system.z80_ram[..],
            };
            memory[offset..offset + poke.bytes.len()].copy_from_slice(&poke.bytes);
            *next += 1;
        }
    }

    fn invalid(message: String) -> Failure {
        Failure { code: "input_replay_invalid".into(), message, remediation:
            Some("use ordered events with frame, port (0..3; 2-3 are J-Cart), and buttons from up/down/left/right/a/b/c/start; [] releases all buttons".into()) }
    }

    fn validate(&self) -> Result<(), Failure> {
        if self.events.len() > 10000 {
            return Err(Self::invalid("replay must have <= 10000 events".into()));
        }
        if self.pokes.len() > 1000 {
            return Err(Self::invalid("replay must have <= 1000 pokes".into()));
        }
        for (i, poke) in self.pokes.iter().enumerate() {
            poke.offset().map_err(Self::invalid)?;
            if i > 0 && poke.frame < self.pokes[i - 1].frame {
                return Err(Self::invalid("pokes must be ordered by frame".into()));
            }
        }
        let mut previous = None;
        let mut ports_at_frame = 0u8;
        for event in &self.events {
            if event.mask().count_ones() as usize != event.buttons.len() {
                return Err(Self::invalid("duplicate buttons in one event".into()));
            }
            if event.port > 3 {
                return Err(Self::invalid("port must be 0..3 (2 and 3 are J-Cart)".into()));
            }
            if previous.is_some_and(|frame| event.frame < frame) {
                return Err(Self::invalid("events must be ordered by frame".into()));
            }
            if previous != Some(event.frame) {
                ports_at_frame = 0;
            }
            if ports_at_frame & (1 << event.port) != 0 {
                return Err(Self::invalid(
                    "a port may have only one event per frame".into(),
                ));
            }
            ports_at_frame |= 1 << event.port;
            previous = Some(event.frame);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ambiguous_or_out_of_order_events_but_allows_two_ports() {
        for text in [
            r#"{"events":[{"frame":2,"port":4,"buttons":[]}]}"#,
            r#"{"events":[{"frame":2,"buttons":[]},{"frame":1,"buttons":[]}]}"#,
            r#"{"events":[{"frame":2,"buttons":[]},{"frame":2,"buttons":["a"]}]}"#,
        ] {
            let replay: Replay = serde_json::from_str(text).unwrap();
            assert!(replay.validate().is_err());
        }
        let replay: Replay = serde_json::from_str(
            r#"{"events":[{"frame":2,"buttons":["a","start"]},{"frame":2,"port":1,"buttons":[]}]}"#,
        )
        .unwrap();
        assert!(replay.validate().is_ok());
        assert_eq!(replay.events[0].mask(), 0x90);
    }
}
