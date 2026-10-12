//! Deterministic controller replay, indexed by completed frames (zero-based).
use crate::envelope::Failure;
use mdcore::input::Button;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputEvent {
    pub frame: u64,
    #[serde(default)]
    pub port: usize,
    pub buttons: Vec<Button>,
}

impl InputEvent {
    pub fn mask(&self) -> u8 {
        self.buttons.iter().fold(0, |mask, b| mask | b.mask())
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    pub events: Vec<InputEvent>,
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

    fn invalid(message: String) -> Failure {
        Failure { code: "input_replay_invalid".into(), message, remediation:
            Some("use ordered events with frame, port (0 or 1), and buttons from up/down/left/right/a/b/c/start; [] releases all buttons".into()) }
    }

    fn validate(&self) -> Result<(), Failure> {
        if self.events.len() > 10000 {
            return Err(Self::invalid("replay must have <= 10000 events".into()));
        }
        let mut previous = None;
        let mut ports_at_frame = 0u8;
        for event in &self.events {
            if event.mask().count_ones() as usize != event.buttons.len() {
                return Err(Self::invalid("duplicate buttons in one event".into()));
            }
            if event.port > 1 {
                return Err(Self::invalid("port must be 0 or 1".into()));
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
            r#"{"events":[{"frame":2,"port":2,"buttons":[]}]}"#,
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
