//! The order work happens in within one logic tick.
//!
//! Game systems join these sets instead of ordering themselves ad hoc, so the
//! port keeps the original's update order. The four sets below are
//! placeholders: once `docs/engine-model.md` records how the original runs a
//! frame (what the vertical blank handler does, what the main loop does and
//! in which order, how object slots are walked), rename and reorder them to
//! match, and cite the evidence here.
use bevy::prelude::*;
use md_bevy::shell::GameSystems;

/// Phases of one logic tick, run in this order inside [`GameSystems::Game`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineSet {
    /// Per-type control code that reads the controller (usually the player).
    Control,
    /// Per-type update code for every other active object.
    Objects,
    /// Trigger detection, then action handlers.
    Triggers,
    /// Camera and scroll logic, when the original runs it per tick.
    Camera,
}

pub struct SchedulePlugin;

impl Plugin for SchedulePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            FixedUpdate,
            (EngineSet::Control, EngineSet::Objects, EngineSet::Triggers, EngineSet::Camera)
                .chain()
                .in_set(GameSystems::Game),
        );
    }
}
