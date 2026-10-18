//! The tick loop: fixed-step logic at the original's rate, controller
//! sampling at tick boundaries, and the sets game systems run in.
//!
//! Gameplay runs in `FixedUpdate`, in [`GameSystems::Game`]. That set is the
//! boundary the developer MCP steps and the headless tests drive, so nothing
//! else needs to know about either.
use crate::controller::{self, ControllerBindings, ControllerState};
use bevy::prelude::*;
use std::time::Duration;

/// The original's video standard: how many frames a second it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    /// 262 lines per frame: 53693175 / (3420 * 262) ≈ 59.92 Hz.
    #[default]
    Ntsc,
    /// 313 lines per frame: 53203424 / (3420 * 313) ≈ 49.70 Hz.
    Pal,
}

impl Region {
    pub fn frame_rate(self) -> f64 {
        match self {
            Self::Ntsc => 53_693_175.0 / (3420.0 * 262.0),
            Self::Pal => 53_203_424.0 / (3420.0 * 313.0),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ntsc" => Some(Self::Ntsc),
            "pal" => Some(Self::Pal),
            _ => None,
        }
    }
}

/// How often game logic runs: once every `frames_per_tick` video frames.
/// Most Mega Drive games run their logic every frame; some wait two.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickRate {
    pub region: Region,
    pub frames_per_tick: u32,
}

impl Default for TickRate {
    fn default() -> Self {
        Self::every_frame(Region::Ntsc)
    }
}

impl TickRate {
    pub const fn every_frame(region: Region) -> Self {
        Self { region, frames_per_tick: 1 }
    }

    pub const fn every_nth_frame(region: Region, frames_per_tick: u32) -> Self {
        assert!(frames_per_tick > 0, "frames_per_tick must be at least 1");
        Self { region, frames_per_tick }
    }

    pub fn hz(self) -> f64 {
        self.region.frame_rate() / self.frames_per_tick as f64
    }

    pub fn period(self) -> Duration {
        Duration::from_secs_f64(1.0 / self.hz())
    }
}

/// Completed logic ticks. `ticking` is false while the game is paused (the
/// controls menu, or the developer MCP between steps).
#[derive(Resource, Default)]
pub struct GameClock {
    pub frame: u64,
    pub ticking: bool,
}

/// Game state the developer `game_status` tool reports under `game`. Insert
/// whatever makes invisible state testable (menu page, position, flags).
#[derive(Resource, Default)]
pub struct GameStatus(pub serde_json::Map<String, serde_json::Value>);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GameSystems {
    Input,
    Game,
    Finish,
}

/// Buttons held by something other than a person: the headless tests and the
/// developer MCP. Merged with physical input.
#[derive(Resource, Default)]
pub struct InjectedInput(pub u16);

impl InjectedInput {
    pub fn set(&mut self, buttons: &[controller::Button]) {
        self.0 = buttons.iter().fold(0, |bits, button| bits | button.mask());
    }
}

/// The tick loop and controller state. Works with `MinimalPlugins` (tests)
/// as well as the full window (the launcher).
pub struct ShellPlugin {
    pub tick: TickRate,
}

impl Plugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.tick)
            .insert_resource(Time::<Fixed>::from_duration(self.tick.period()))
            .init_resource::<crate::assets::AssetRoot>()
            .init_resource::<GameClock>()
            .init_resource::<GameStatus>()
            .init_resource::<InjectedInput>()
            .init_resource::<ControllerBindings>()
            .init_resource::<ControllerState>()
            .configure_sets(
                FixedUpdate,
                (GameSystems::Input, GameSystems::Game, GameSystems::Finish).chain(),
            )
            .configure_sets(
                FixedUpdate,
                GameSystems::Game.run_if(|clock: Res<GameClock>| clock.ticking),
            )
            .add_systems(FixedUpdate, begin_tick.in_set(GameSystems::Input))
            .add_systems(FixedUpdate, finish_tick.in_set(GameSystems::Finish));
    }
}

fn begin_tick(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<(Entity, &Gamepad)>,
    bindings: Res<ControllerBindings>,
    injected: Res<InjectedInput>,
    mut state: ResMut<ControllerState>,
    mut clock: ResMut<GameClock>,
    menu: Option<Res<crate::controls_menu::ControlsMenu>>,
    #[cfg(feature = "dev-mcp")] mut control: Option<ResMut<crate::dev_mcp::Control>>,
) {
    // The controls menu pauses play; developer steps still advance.
    let menu_open = menu.is_some_and(|menu| menu.open);
    clock.ticking = !menu_open;
    #[cfg(not(feature = "dev-mcp"))]
    let developer = 0;
    #[cfg(feature = "dev-mcp")]
    let mut developer = 0;
    #[cfg(feature = "dev-mcp")]
    if let Some(ref mut control) = control {
        control.discard_cancelled_step();
        clock.ticking = (!control.paused && !menu_open) || control.has_step();
        developer = control.buttons;
    }
    if clock.ticking {
        let pad = pads
            .iter()
            .min_by_key(|(entity, _)| entity.to_bits())
            .map(|(_, pad)| pad);
        let physical = match keys {
            Some(keys) if !menu_open => controller::sample(&keys, pad, &bindings),
            _ => 0,
        };
        state.update(physical | injected.0 | developer);
    }
}

fn finish_tick(
    mut clock: ResMut<GameClock>,
    #[cfg(feature = "dev-mcp")] control: Option<ResMut<crate::dev_mcp::Control>>,
) {
    if !clock.ticking {
        return;
    }
    clock.frame += 1;
    #[cfg(feature = "dev-mcp")]
    if let Some(mut control) = control {
        control.finish_tick(clock.frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_follow_the_master_clock() {
        assert!((Region::Ntsc.frame_rate() - 59.9227).abs() < 1e-3);
        assert!((Region::Pal.frame_rate() - 49.7014).abs() < 1e-3);
        assert!((TickRate::every_nth_frame(Region::Ntsc, 2).hz() - 29.9613).abs() < 1e-3);
    }
}
