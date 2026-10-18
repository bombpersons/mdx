//! The demo's player rules: a registered object type moved by the d-pad,
//! with its speed in `assets/demo/player.ron`. No sprites or assets here, so
//! headless tests can run it (see `tests/player.rs`).
use bevy::prelude::*;
use engine::prelude::*;
use serde::Deserialize;

pub const PLAYER: ObjectTypeId = ObjectTypeId(1);

/// Design numbers for the player, from `assets/demo/player.ron`.
#[derive(Resource, Deserialize, Debug, Clone)]
pub struct PlayerTuning {
    /// Pixels per tick.
    pub speed: i32,
    /// Size in pixels; the player stays inside the playfield.
    pub size: IVec2,
    /// The area the player may move in: top-left pixel...
    pub min: IVec2,
    /// ...and the pixel just past its bottom-right corner.
    pub max: IVec2,
}

#[derive(Component, Default)]
pub struct Player;

/// Start toggles this; while paused the player does not move.
#[derive(Resource, Default)]
pub struct DemoPaused(pub bool);

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TuningPlugin::<PlayerTuning>::new("demo/player.ron"))
            .init_resource::<DemoPaused>()
            .register_object_type(
                PLAYER,
                ObjectTypeInfo::new("Player", |e, _| {
                    e.insert(Player);
                })
                .with_tuning::<PlayerTuning>(),
            )
            .add_systems(FixedUpdate, (pause, walk).chain().in_set(EngineSet::Control));
    }
}

fn pause(pad: Res<ControllerState>, mut paused: ResMut<DemoPaused>) {
    if pad.pressed(Button::Start) {
        paused.0 = !paused.0;
    }
}

fn walk(
    pad: Res<ControllerState>,
    paused: Res<DemoPaused>,
    tuning: Res<PlayerTuning>,
    mut players: Query<&mut Position, With<Player>>,
) {
    if paused.0 {
        return;
    }
    let x = i32::from(pad.held(Button::Right)) - i32::from(pad.held(Button::Left));
    let y = i32::from(pad.held(Button::Down)) - i32::from(pad.held(Button::Up));
    for mut position in &mut players {
        let moved = position.0 + IVec2::new(x, y) * tuning.speed;
        position.0 = moved.clamp(tuning.min, tuning.max - tuning.size);
    }
}
