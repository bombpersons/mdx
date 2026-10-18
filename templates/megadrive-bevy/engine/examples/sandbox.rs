//! The sandbox: a second, tiny game built only from `engine` and the
//! hand-authored files in `engine/assets/sandbox/`. It must never depend on
//! `game/` or on anything read from the ROM. If a feature of the port cannot
//! be used here without reaching into `game/`, the engine boundary is in the
//! wrong place: fix the boundary, not the sandbox.
//!
//! cargo run -p engine --example sandbox
//! cargo run -p engine --features dev-mcp --example sandbox -- --mcp --headless --software
//!
//! One room (a tile plane), one controllable character, one object type with
//! a trivial controller (a bobbing marker) and one trigger (it recolours the
//! floor by editing the palette). Every value comes from `room.ron` or
//! `walker.ron`.
use bevy::prelude::*;
use bevy::sprite::Anchor;
use engine::launcher::{self, Game};
use engine::prelude::*;
use serde::Deserialize;
use std::sync::Arc;

const WALKER: ObjectTypeId = ObjectTypeId(1);
const BOBBER: ObjectTypeId = ObjectTypeId(2);
const RECOLOUR: ActionId = ActionId(1);

/// The sandbox's room: palettes, a walled tile room and level data.
#[derive(Deserialize)]
struct Room {
    palettes: Palettes,
    tileset: String,
    size: UVec2,
    floor: TileRef,
    wall: TileRef,
    walker: String,
    bobber: String,
    level: LevelData,
}

#[derive(Resource, Deserialize)]
struct WalkerTuning {
    /// Pixels per tick.
    speed: i32,
}

#[derive(Component, Default)]
#[require(TriggerActivator)]
struct Walker;

/// Bobs up and down by `params[0]` pixels, one pixel per tick.
#[derive(Component)]
struct Bobber {
    base_y: i32,
    amplitude: i32,
}

/// The indexed art the sandbox's objects are drawn with.
#[derive(Resource)]
struct Art {
    walker: Arc<IndexedImage>,
    bobber: Arc<IndexedImage>,
}

fn main() {
    launcher::run(Game {
        name: "engine-sandbox",
        title: "Engine sandbox",
        region: Region::Ntsc,
        frames_per_tick: 1,
        screen: UVec2::new(320, 224),
        assets: concat!(env!("CARGO_MANIFEST_DIR"), "/assets"),
        add: |app| {
            app.add_plugins((EnginePlugin, TuningPlugin::<WalkerTuning>::new("sandbox/walker.ron")))
                .register_object_type(
                    WALKER,
                    ObjectTypeInfo::new("Walker", |e, _| {
                        e.insert(Walker);
                    })
                    .with_tuning::<WalkerTuning>(),
                )
                .register_object_type(
                    BOBBER,
                    ObjectTypeInfo::new("Bobber", |e, p| {
                        let amplitude = p.params.first().copied().unwrap_or(0);
                        e.insert(Bobber { base_y: p.position.y, amplitude });
                    }),
                )
                .register_action(RECOLOUR, "recolour", recolour)
                .add_systems(Startup, setup)
                .add_systems(FixedUpdate, walk.in_set(EngineSet::Control))
                .add_systems(FixedUpdate, bob.in_set(EngineSet::Objects))
                .add_systems(Update, (dress, show));
        },
    });
}

fn setup(mut commands: Commands, root: Res<AssetRoot>) {
    let room: Room = root.load_ron("sandbox/room.ron").unwrap_or_else(|e| panic!("sandbox room: {e}"));
    let load = |path: &str| Arc::new(IndexedImage::load(&root, path).unwrap_or_else(|e| panic!("{e}")));
    let mut map = TileMap::new(room.size.x, room.size.y);
    for y in 0..room.size.y {
        for x in 0..room.size.x {
            let edge = x == 0 || y == 0 || x == room.size.x - 1 || y == room.size.y - 1;
            map.set(x, y, if edge { room.wall } else { room.floor });
        }
    }
    let plane = TilePlane::new(load(&room.tileset), map).unwrap_or_else(|e| panic!("sandbox room: {e}"));
    commands.spawn((Name::new("Room"), plane));
    commands.insert_resource(room.palettes);
    commands.insert_resource(Art { walker: load(&room.walker), bobber: load(&room.bobber) });
    commands.queue(SpawnLevel(room.level));
}

fn walk(pad: Res<ControllerState>, tuning: Res<WalkerTuning>, mut q: Query<&mut Position, With<Walker>>) {
    let x = i32::from(pad.held(Button::Right)) - i32::from(pad.held(Button::Left));
    let y = i32::from(pad.held(Button::Down)) - i32::from(pad.held(Button::Up));
    for mut position in &mut q {
        position.0 += IVec2::new(x, y) * tuning.speed;
    }
}

fn bob(clock: Res<GameClock>, mut q: Query<(&Bobber, &mut Position)>) {
    for (bobber, mut position) in &mut q {
        // A triangle wave: up `amplitude` pixels and back, one pixel per tick.
        let period = (bobber.amplitude * 2).max(1) as u64;
        let phase = (clock.frame % period) as i32;
        position.0.y = bobber.base_y - phase.min(bobber.amplitude * 2 - phase);
    }
}

/// args: pairs of [line, colour, r, g, b] to write into the palettes.
fn recolour(In(call): In<ActionCall>, mut palettes: ResMut<Palettes>) {
    for entry in call.args.chunks_exact(5) {
        let [line, index, r, g, b] = [0, 1, 2, 3, 4].map(|i| entry[i].clamp(0, 15) as usize);
        if let Ok(colour) = MdColor::try_from((r as u8, g as u8, b as u8)) {
            palettes.lines[line.min(3)].0[index] = colour;
        }
    }
}

fn dress(
    mut commands: Commands,
    art: Res<Art>,
    walkers: Query<(Entity, &Position), Added<Walker>>,
    bobbers: Query<(Entity, &Position), Added<Bobber>>,
) {
    let walkers = walkers.iter().map(|(e, p)| (e, p, &art.walker));
    for (entity, position, image) in walkers.chain(bobbers.iter().map(|(e, p)| (e, p, &art.bobber))) {
        commands.entity(entity).insert((
            IndexedSprite { image: image.clone(), palette: 0 },
            Anchor::TOP_LEFT,
            Transform::from_xyz(0.0, 0.0, Depth::SPRITES_LOW),
            ScreenPosition(position.0),
        ));
    }
}

/// The room fills the screen, so level pixels are screen pixels.
fn show(mut q: Query<(&Position, &mut ScreenPosition), Changed<Position>>) {
    for (position, mut on_screen) in &mut q {
        on_screen.0 = position.0;
    }
}
