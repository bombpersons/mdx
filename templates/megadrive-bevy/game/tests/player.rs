//! Headless tests of the demo player's rules. A port's tests look the same:
//! the files in `tests/data/` come from mdx (`boot` with `input` and
//! `samples`, see `md_bevy::testing::trajectory`), recorded from the original
//! game. These ones are hand-written examples of the formats, since the
//! demo has no original.
use bevy::prelude::*;
use engine::prelude::*;
use engine::testing::{Alignment, Replay, TickApp, Trajectory, compare, headless_app};
use game::demo::{DemoLevel, player::{Player, PlayerPlugin}};

/// The rules plugins and the demo level's placements, without presentation.
fn app() -> (App, Entity) {
    let mut app = headless_app(TickRate::every_nth_frame(game::REGION, game::FRAMES_PER_TICK));
    app.add_plugins((EnginePlugin, PlayerPlugin));
    let level = DemoLevel::load(app.world().resource::<AssetRoot>()).unwrap();
    SpawnLevel(level.level).apply(app.world_mut());
    let player = app.world_mut().query_filtered::<Entity, With<Player>>().single(app.world()).unwrap();
    (app, player)
}

fn position(app: &App, player: Entity) -> IVec2 {
    app.world().get::<Position>(player).unwrap().0
}

#[test]
fn walks_two_pixels_a_tick_and_stops_at_the_edge() {
    let (mut app, player) = app();
    let start = position(&app, player);
    app.run_ticks(10, &[Button::Right]);
    assert_eq!(position(&app, player), start + IVec2::new(20, 0));
    app.run_ticks(200, &[Button::Right, Button::Down]);
    assert_eq!(position(&app, player), IVec2::new(320 - 12, 224 - 16));
}

#[test]
fn start_pauses_movement() {
    let (mut app, player) = app();
    app.run_tick(&[Button::Start]);
    let paused_at = position(&app, player);
    app.run_ticks(5, &[Button::Left]);
    assert_eq!(position(&app, player), paused_at);
}

#[test]
fn matches_the_recorded_trajectory() {
    let align = Alignment::new(game::FRAMES_PER_TICK, 0);
    let original = Trajectory::load_mdx_samples("tests/data/walk.samples.csv", align).unwrap();
    let inputs = Replay::load("tests/data/walk.input.json").unwrap().tick_inputs(align, original.ticks() - 1).unwrap();
    let (mut app, player) = app();
    // The recording's starting state.
    let start = IVec2::new(original.get(0, "x").unwrap() as i32, original.get(0, "y").unwrap() as i32);
    app.world_mut().get_mut::<Position>(player).unwrap().0 = start;
    let rebuild = Trajectory::record(&mut app, &inputs, &["x", "y"], |world| {
        let p = world.get::<Position>(player).unwrap().0;
        vec![p.x as f64, p.y as f64]
    });
    compare(&original, &rebuild, &[("x", 0.0), ("y", 0.0)]).unwrap().assert_passed();
}
