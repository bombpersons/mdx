//! Level data in the engine's own format: what a level places and which
//! triggers it has. Importers in `tools/` write it from the original's
//! tables; people can write it by hand (RON).
//!
//! Extend it with what the original's levels actually hold (tile planes as
//! [`md_bevy::look::TileMap`]s, collision, camera bounds, event lists),
//! keeping units normal (pixels, ticks) and conversions in the importer.
use crate::objects::{Placement, Position, spawn_placement};
use crate::triggers::{ActionCall, TriggerArea};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LevelData {
    #[serde(default)]
    pub objects: Vec<Placement>,
    #[serde(default)]
    pub triggers: Vec<TriggerPlacement>,
}

/// A trigger rectangle: top-left `position` and `size`, in level pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriggerPlacement {
    pub position: IVec2,
    pub size: IVec2,
    pub call: ActionCall,
    #[serde(default)]
    pub once: bool,
}

/// A command that spawns a level's objects (through the type registry) and
/// trigger areas: `commands.queue(SpawnLevel(level))`.
pub struct SpawnLevel(pub LevelData);

impl Command for SpawnLevel {
    type Out = ();

    fn apply(self, world: &mut World) {
        for placement in &self.0.objects {
            spawn_placement(world, placement);
        }
        for trigger in self.0.triggers {
            world.spawn((
                Name::new("Trigger"),
                Position(trigger.position),
                TriggerArea::new(trigger.size, trigger.call, trigger.once),
            ));
        }
    }
}
