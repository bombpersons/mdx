//! Minimal example controls menu, drawn inside the 320×224 game framebuffer in
//! a Mega Drive style: palette-snapped colours, a bordered window with a hard
//! shadow, and the 5×7 bitmap font from `pixel_font`. Move it with `POSITION`, restyle it with the
//! constants below, or delete this module and drive `rebind::Rebind` from your
//! own menus.
//!
//! F1 or a gamepad's Select toggles it; the simulation pauses while it is open.
//! Its navigation keys are fixed, so a bad binding can never lock you out.
use crate::controller::{Button, ControllerBindings};
use crate::pixel_font::{self, ADVANCE};
use crate::rebind::{Rebind, key_name, pad_name};
use bevy::prelude::*;
use bevy::sprite::Anchor;

/// Window top-left in game pixels, relative to the screen centre.
const POSITION: Vec2 = Vec2::new(-80.0, 64.0);
const SIZE: Vec2 = Vec2::new(160.0, 128.0);
const LINE: f32 = 10.0;
const PADDING: Vec2 = Vec2::new(8.0, 8.0);
/// Characters that fit on one line; longer device names are clipped.
const COLUMNS: usize = ((SIZE.x - 2.0 * PADDING.x) as u32 / ADVANCE) as usize;
const WINDOW: Color = md(0, 0, 3);
const BORDER: Color = md(6, 6, 7);
const SHADOW: Color = md(0, 0, 0);
const TEXT: Color = md(7, 7, 7);
const DIM: Color = md(4, 5, 6);
const SELECTED: Color = md(7, 6, 0);

/// A colour from the Mega Drive's 3-bit-per-channel palette (levels 0..7).
const fn md(r: u8, g: u8, b: u8) -> Color {
    Color::srgb_u8(r * 36, g * 36, b * 36)
}

/// Rows: one per Mega Drive button, then reset.
const RESET: usize = Button::ALL.len();
const ROWS: usize = RESET + 1;

#[derive(Resource, Default)]
pub struct ControlsMenu {
    pub open: bool,
    row: usize,
}

#[derive(Component)]
struct MenuRoot;
/// A text line and the string its image currently shows.
#[derive(Component)]
struct MenuLine(usize, String);

pub struct ControlsMenuPlugin;

impl Plugin for ControlsMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControlsMenu>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (navigate, redraw).chain());
    }
}

fn spawn(mut commands: Commands) {
    let rect = |color, offset: Vec2, grow: f32, z| {
        (
            Sprite::from_color(color, SIZE + Vec2::splat(grow * 2.0)),
            Anchor::TOP_LEFT,
            Transform::from_xyz(offset.x - grow, offset.y + grow, z),
        )
    };
    commands
        .spawn((
            MenuRoot,
            Transform::from_translation(POSITION.extend(50.0)),
            Visibility::Hidden,
        ))
        .with_children(|menu| {
            menu.spawn(rect(SHADOW, Vec2::new(3.0, -3.0), 1.0, 0.0));
            menu.spawn(rect(BORDER, Vec2::ZERO, 1.0, 0.1));
            menu.spawn(rect(WINDOW, Vec2::ZERO, 0.0, 0.2));
            // Line 0 is the title, then a gap, the rows, a gap and the hint.
            for line in 0..ROWS + 2 {
                let gap = if line == 0 { 0.0 } else if line > ROWS { 2.0 } else { 1.0 };
                let y = (PADDING.y + LINE * (line as f32 + gap * 0.5)).round();
                menu.spawn((
                    MenuLine(line, String::new()),
                    Sprite::default(),
                    Anchor::TOP_LEFT,
                    Transform::from_xyz(PADDING.x, -y, 1.0),
                ));
            }
        });
}

fn navigate(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut menu: ResMut<ControlsMenu>,
    mut rebind: ResMut<Rebind>,
    mut bindings: ResMut<ControllerBindings>,
) {
    // A press that completed a capture this frame belongs to the capture.
    if rebind.capturing().is_some() || rebind.is_changed() {
        return;
    }
    let pad = |button| pads.iter().any(|pad| pad.just_pressed(button));
    let toggle = keys.just_pressed(KeyCode::F1) || pad(GamepadButton::Select);
    if !menu.open {
        menu.open = toggle;
        return;
    }
    if toggle || keys.just_pressed(KeyCode::Escape) || pad(GamepadButton::East) {
        menu.open = false;
    } else if keys.just_pressed(KeyCode::ArrowUp) || pad(GamepadButton::DPadUp) {
        menu.row = (menu.row + ROWS - 1) % ROWS;
    } else if keys.just_pressed(KeyCode::ArrowDown) || pad(GamepadButton::DPadDown) {
        menu.row = (menu.row + 1) % ROWS;
    } else if keys.just_pressed(KeyCode::Enter) || pad(GamepadButton::South) {
        match menu.row {
            RESET => rebind.reset(&mut bindings),
            row => rebind.start(Button::ALL[row]),
        }
    }
}

fn redraw(
    menu: Res<ControlsMenu>,
    rebind: Res<Rebind>,
    bindings: Res<ControllerBindings>,
    mut root: Query<&mut Visibility, With<MenuRoot>>,
    mut lines: Query<(&mut MenuLine, &mut Sprite)>,
    mut images: ResMut<Assets<Image>>,
) {
    for mut visibility in &mut root {
        *visibility = if menu.open { Visibility::Inherited } else { Visibility::Hidden };
    }
    if !menu.open {
        return;
    }
    for (mut line, mut sprite) in &mut lines {
        let row = line.0.wrapping_sub(1);
        let selected = row == menu.row;
        let cursor = if selected { ">" } else { " " };
        let label = |button: Button| format!("{button:?}").to_uppercase();
        let (value, tint) = match line.0 {
            0 => ("CONTROLS".to_string(), TEXT),
            n if n == ROWS + 1 => match rebind.capturing() {
                Some(_) => ("PRESS KEY OR BUTTON".into(), SELECTED),
                None if rebind.status.is_empty() => ("ENTER:SET  ESC:EXIT".into(), DIM),
                None => (rebind.status.to_uppercase(), DIM),
            },
            _ if row == RESET => (format!("{cursor} RESET DEFAULTS"), TEXT),
            _ if rebind.capturing() == Some(Button::ALL[row]) => {
                (format!("{cursor} {:<6}...", label(Button::ALL[row])), SELECTED)
            }
            _ => (
                format!(
                    "{cursor} {:<6}{:<6}{}",
                    label(Button::ALL[row]),
                    key_name(bindings.keyboard[row]).to_uppercase().chars().take(5).collect::<String>(),
                    pad_name(bindings.gamepad[row]).to_uppercase()
                ),
                TEXT,
            ),
        };
        let value: String = value.chars().take(COLUMNS).collect();
        if line.1 != value {
            sprite.image = images.add(pixel_font::text_image(&value));
            line.1 = value;
        }
        sprite.color = if selected && line.0 != 0 && line.0 <= ROWS { SELECTED } else { tint };
    }
}
