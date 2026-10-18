//! Edit bindings here; game logic only consumes Mega Drive buttons.
use bevy::prelude::*;

/// The names (`up`, `a`, `start`...) are the ones mdx's controller replays use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    C,
    Start,
}

impl Button {
    pub const ALL: [Self; 8] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::A,
        Self::B,
        Self::C,
        Self::Start,
    ];
    pub fn mask(self) -> u16 {
        1 << self as u8
    }
}

#[derive(Resource, Default)]
pub struct ControllerState {
    held: u16,
    previous: u16,
}

impl ControllerState {
    pub fn update(&mut self, held: u16) {
        self.previous = self.held;
        self.held = held;
    }
    pub fn held(&self, button: Button) -> bool {
        self.held & button.mask() != 0
    }
    pub fn pressed(&self, button: Button) -> bool {
        (self.held & !self.previous) & button.mask() != 0
    }
    pub fn released(&self, button: Button) -> bool {
        (!self.held & self.previous) & button.mask() != 0
    }
    pub fn buttons(&self) -> Vec<Button> {
        Button::ALL.into_iter().filter(|b| self.held(*b)).collect()
    }
}

/// Indexed by `Button as usize`. `rebind` saves and restores this as JSON.
#[derive(Resource, Clone, serde::Serialize, serde::Deserialize)]
pub struct ControllerBindings {
    pub keyboard: [KeyCode; 8],
    pub gamepad: [GamepadButton; 8],
    pub stick_deadzone: f32,
}

impl Default for ControllerBindings {
    fn default() -> Self {
        Self {
            keyboard: [
                KeyCode::ArrowUp,
                KeyCode::ArrowDown,
                KeyCode::ArrowLeft,
                KeyCode::ArrowRight,
                KeyCode::KeyZ,
                KeyCode::KeyX,
                KeyCode::KeyC,
                KeyCode::Enter,
            ],
            gamepad: [
                GamepadButton::DPadUp,
                GamepadButton::DPadDown,
                GamepadButton::DPadLeft,
                GamepadButton::DPadRight,
                GamepadButton::West,
                GamepadButton::South,
                GamepadButton::East,
                GamepadButton::Start,
            ],
            stick_deadzone: 0.25,
        }
    }
}

pub fn sample(
    keys: &ButtonInput<KeyCode>,
    pad: Option<&Gamepad>,
    bindings: &ControllerBindings,
) -> u16 {
    let mut bits = 0;
    for (index, button) in Button::ALL.into_iter().enumerate() {
        if keys.pressed(bindings.keyboard[index])
            || pad.is_some_and(|pad| pad.pressed(bindings.gamepad[index]))
        {
            bits |= button.mask();
        }
    }
    if let Some(pad) = pad {
        let stick = pad.left_stick();
        if stick.x < -bindings.stick_deadzone {
            bits |= Button::Left.mask();
        }
        if stick.x > bindings.stick_deadzone {
            bits |= Button::Right.mask();
        }
        if stick.y < -bindings.stick_deadzone {
            bits |= Button::Down.mask();
        }
        if stick.y > bindings.stick_deadzone {
            bits |= Button::Up.mask();
        }
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edges_are_simulation_ticks_not_window_frames() {
        let mut pad = ControllerState::default();
        pad.update(Button::A.mask());
        assert!(pad.pressed(Button::A));
        assert!(pad.held(Button::A));
        pad.update(Button::A.mask());
        assert!(!pad.pressed(Button::A));
        pad.update(0);
        assert!(pad.released(Button::A));
        assert!(!pad.held(Button::A));
    }
    #[test]
    fn default_keyboard_is_genesis_zxc_enter_and_arrows() {
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyX);
        keys.press(KeyCode::ArrowRight);
        assert_eq!(
            sample(&keys, None, &ControllerBindings::default()),
            Button::B.mask() | Button::Right.mask()
        );
    }
}
