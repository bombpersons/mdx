//! Replace this module with your native game's ordinary Bevy systems.
use crate::{
    controller::{Button, ControllerState},
    screen::Screen,
};
use bevy::prelude::*;

#[derive(Component)]
pub(crate) struct Player;
#[derive(Component)]
pub(crate) struct Indicator(Button);
#[derive(Resource, Default)]
pub(crate) struct DemoState {
    paused: bool,
}

pub fn setup(mut commands: Commands, screen: Res<Screen>) {
    commands.init_resource::<DemoState>();
    let width = screen.width as f32;
    let height = screen.height as f32;
    for row in 0..screen.height / 16 {
        for column in 0..screen.width / 16 {
            let c = if (row + column) % 2 == 0 { 0.12 } else { 0.15 };
            commands.spawn((
                Sprite::from_color(Color::srgb(c, c + 0.04, c + 0.09), Vec2::splat(16.)),
                Transform::from_xyz(
                    column as f32 * 16. - width / 2. + 8.,
                    row as f32 * 16. - height / 2. + 8.,
                    0.,
                ),
            ));
        }
    }
    commands.spawn((
        Sprite::from_color(Color::srgb(0.35, 0.85, 1.), Vec2::new(12., 16.)),
        Transform::from_xyz(0., 0., 1.),
        Player,
    ));
    for (index, button) in Button::ALL.into_iter().enumerate() {
        commands.spawn((
            Sprite::from_color(Color::srgb(0.3, 0.35, 0.4), Vec2::splat(10.)),
            Transform::from_xyz(-56. + index as f32 * 16., -height / 2. + 12., 2.),
            Indicator(button),
        ));
    }
}

pub fn tick(
    pad: Res<ControllerState>,
    screen: Res<Screen>,
    mut state: ResMut<DemoState>,
    mut player: Query<&mut Transform, With<Player>>,
    mut indicators: Query<(&Indicator, &mut Sprite)>,
) {
    if pad.pressed(Button::Start) {
        state.paused = !state.paused;
    }
    for (indicator, mut sprite) in &mut indicators {
        sprite.color = if pad.held(indicator.0) {
            Color::srgb(1., 0.7, 0.25)
        } else if pad.released(indicator.0) {
            Color::srgb(0.35, 0.4, 0.45)
        } else {
            Color::srgb(0.3, 0.35, 0.4)
        };
    }
    if state.paused {
        return;
    }
    let x = i32::from(pad.held(Button::Right)) - i32::from(pad.held(Button::Left));
    let y = i32::from(pad.held(Button::Up)) - i32::from(pad.held(Button::Down));
    for mut player in &mut player {
        player.translation.x = (player.translation.x + 2. * x as f32).clamp(
            -(screen.width as f32) / 2. + 6.,
            screen.width as f32 / 2. - 6.,
        );
        player.translation.y = (player.translation.y + 2. * y as f32).clamp(
            -(screen.height as f32) / 2. + 8.,
            screen.height as f32 / 2. - 8.,
        );
    }
}
