//! Control remapping without any UI: call `Rebind::start(button)` and the next
//! key or gamepad button pressed is bound to it (swapping with whichever button
//! already used it) and saved. Build your own menu on top, or use the minimal
//! example in `controls_menu.rs`.
//!
//! Saved bindings live in `bindings.json` under the platform config directory
//! (see `config_path`) and are loaded at startup.
use crate::controller::{Button, ControllerBindings};
use bevy::prelude::*;
use std::path::PathBuf;

#[derive(Resource, Default)]
pub struct Rebind {
    capturing: Option<Button>,
    /// Outcome of the last capture, reset or save, for display.
    pub status: String,
}

// `cancel` and friends are API for custom menus; the example uses a subset.
#[allow(dead_code)]
impl Rebind {
    /// Bind the next key or pad button pressed to `button`. Escape cancels.
    pub fn start(&mut self, button: Button) {
        self.capturing = Some(button);
        self.status.clear();
    }
    pub fn cancel(&mut self) {
        self.capturing = None;
    }
    /// The button waiting for a press, if any. Suppress your own menu input
    /// while this is `Some`, since the press belongs to the capture.
    pub fn capturing(&self) -> Option<Button> {
        self.capturing
    }
    /// Restore `ControllerBindings::default()` and save it.
    pub fn reset(&mut self, bindings: &mut ControllerBindings) {
        *bindings = ControllerBindings::default();
        self.status = save(bindings);
    }
}

pub struct RebindPlugin;

impl Plugin for RebindPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rebind>()
            .add_systems(Startup, load)
            .add_systems(PreUpdate, capture.after(bevy::input::InputSystems));
    }
}

/// `$XDG_CONFIG_HOME`/`~/.config` (Linux), `%APPDATA%` (Windows) or
/// `~/Library/Application Support` (macOS), then the package name.
pub fn config_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|dir| dir.join(env!("CARGO_PKG_NAME")).join("bindings.json"))
}

fn load(mut bindings: ResMut<ControllerBindings>) {
    let Some(path) = config_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    match serde_json::from_str(&text) {
        Ok(saved) => *bindings = saved,
        Err(error) => warn!("ignoring unreadable {}: {error}", path.display()),
    }
}

fn save(bindings: &ControllerBindings) -> String {
    let Some(path) = config_path() else {
        return "Not saved (no config directory)".into();
    };
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, serde_json::to_string_pretty(bindings).unwrap()));
    match written {
        Ok(()) => "Saved".into(),
        Err(error) => format!("Not saved: {error}"),
    }
}

fn capture(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut rebind: ResMut<Rebind>,
    mut bindings: ResMut<ControllerBindings>,
) {
    let Some(button) = rebind.capturing else { return };
    let index = button as usize;
    if keys.just_pressed(KeyCode::Escape) {
        rebind.capturing = None;
        rebind.status = "Cancelled".into();
    } else if let Some(key) = keys.get_just_pressed().next() {
        assign_key(&mut bindings, index, *key);
        rebind.capturing = None;
        rebind.status = save(&bindings);
    } else if let Some(pad_button) = pads.iter().find_map(|pad| pad.get_just_pressed().next()) {
        assign_pad(&mut bindings, index, *pad_button);
        rebind.capturing = None;
        rebind.status = save(&bindings);
    }
}

/// Bind `key` at `index`, swapping with whichever button already used it.
pub fn assign_key(bindings: &mut ControllerBindings, index: usize, key: KeyCode) {
    if let Some(other) = bindings.keyboard.iter().position(|k| *k == key) {
        bindings.keyboard[other] = bindings.keyboard[index];
    }
    bindings.keyboard[index] = key;
}

/// Bind `pad_button` at `index`, swapping with whichever button already used it.
pub fn assign_pad(bindings: &mut ControllerBindings, index: usize, pad_button: GamepadButton) {
    if let Some(other) = bindings.gamepad.iter().position(|b| *b == pad_button) {
        bindings.gamepad[other] = bindings.gamepad[index];
    }
    bindings.gamepad[index] = pad_button;
}

/// Short label: `KeyZ` → `Z`, `Digit1` → `1`, `ArrowUp` → `Up`.
pub fn key_name(key: KeyCode) -> String {
    let debug = format!("{key:?}");
    for prefix in ["Arrow", "Key", "Digit"] {
        if let Some(rest) = debug.strip_prefix(prefix) {
            if prefix == "Arrow" || rest.len() == 1 {
                return rest.into();
            }
        }
    }
    debug
}

/// Short label: `DPadUp` → `Pad Up`, `South` → `Pad South`.
pub fn pad_name(button: GamepadButton) -> String {
    let debug = format!("{button:?}");
    format!("Pad {}", debug.strip_prefix("DPad").unwrap_or(&debug))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigning_a_used_key_swaps_rather_than_duplicating() {
        let mut bindings = ControllerBindings::default();
        assign_key(&mut bindings, Button::A as usize, KeyCode::KeyX);
        assert_eq!(bindings.keyboard[Button::A as usize], KeyCode::KeyX);
        assert_eq!(bindings.keyboard[Button::B as usize], KeyCode::KeyZ);
        assign_pad(&mut bindings, Button::Start as usize, GamepadButton::South);
        assert_eq!(bindings.gamepad[Button::Start as usize], GamepadButton::South);
        assert_eq!(bindings.gamepad[Button::B as usize], GamepadButton::Start);
    }

    #[test]
    fn bindings_round_trip_through_json_and_labels_are_short() {
        let bindings = ControllerBindings::default();
        let text = serde_json::to_string(&bindings).unwrap();
        let back: ControllerBindings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.keyboard, bindings.keyboard);
        assert_eq!(back.gamepad, bindings.gamepad);
        assert_eq!(key_name(KeyCode::KeyZ), "Z");
        assert_eq!(key_name(KeyCode::ArrowUp), "Up");
        assert_eq!(key_name(KeyCode::Enter), "Enter");
        assert_eq!(pad_name(GamepadButton::DPadLeft), "Pad Left");
    }
}
