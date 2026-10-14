//! Shared resumable-state transport for execution operations.
use crate::{
    artifacts::{self, Artifact},
    context::Context,
    envelope::Failure,
};
use mdcore::{cart_hw::ConsoleRegion, system::MachineState, Cartridge, System};

/// Console region chosen at power-on. `auto` follows the header like the
/// reference; `europe` runs PAL timing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RegionChoice {
    #[default]
    Auto,
    Usa,
    Japan,
    Europe,
}

pub fn load(
    cart: Cartridge,
    source: Option<&str>,
    region: RegionChoice,
    context: &Context,
) -> Result<System, Failure> {
    let Some(source) = source else {
        let mut system = System::new(cart);
        match region {
            RegionChoice::Auto => {}
            RegionChoice::Usa => system.set_region(ConsoleRegion::Usa),
            RegionChoice::Japan => system.set_region(ConsoleRegion::JapanNtsc),
            RegionChoice::Europe => system.set_region(ConsoleRegion::Europe),
        }
        return Ok(system);
    };
    if region != RegionChoice::Auto {
        return Err(artifacts::failure(
            "invalid_arguments",
            "region is chosen at power-on; a machine state already carries it",
        ));
    }
    let path = artifacts::resolve(&context.store, source)?;
    let bytes = artifacts::read_bounded(&path, 16 * 1024 * 1024)?;
    let state: MachineState = serde_json::from_slice(&bytes)
        .map_err(|e| artifacts::failure("invalid_state", e.to_string()))?;
    System::from_state(cart, state).map_err(|e| artifacts::failure("invalid_state", e))
}

pub fn save(
    system: &System,
    requested: bool,
    context: &Context,
) -> Result<Option<Artifact>, Failure> {
    if !requested {
        return Ok(None);
    }
    let state = system
        .save_state()
        .map_err(|e| artifacts::failure("invalid_state", e))?;
    let bytes = serde_json::to_vec(&state)
        .map_err(|e| artifacts::failure("state_encode_failed", e.to_string()))?;
    artifacts::put(&context.store, &bytes).map(Some)
}
