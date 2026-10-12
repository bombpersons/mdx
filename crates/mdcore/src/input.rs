//! Three-button Mega Drive controller pins. Pressed buttons are active-low.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
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
    pub fn mask(self) -> u8 {
        1 << self as u8
    }
}

pub(crate) fn read_pad(data: u8, control: u8, pressed: u8) -> u8 {
    let th = control & 0x40 == 0 || data & 0x40 != 0;
    let input = if th {
        let low = pressed & 0x0f | ((pressed >> 1) & 0x30);
        0x7f & !low
    } else {
        let low = pressed & 3 | (pressed & Button::A.mask()) | ((pressed >> 2) & 0x20);
        0x33 & !low
    };
    let output_mask = control | 0x80;
    (data & output_mask) | (input & !output_mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn th_selects_bc_or_a_start_and_low_phase_identifies_the_pad() {
        assert_eq!(read_pad(0x40, 0x40, 0), 0x7f);
        assert_eq!(read_pad(0, 0x40, 0), 0x33);
        assert_eq!(read_pad(0x40, 0x40, Button::Start.mask()), 0x7f);
        assert_eq!(read_pad(0, 0x40, Button::Start.mask()), 0x13);
        assert_eq!(read_pad(0x40, 0x40, Button::B.mask()), 0x6f);
        assert_eq!(read_pad(0, 0x40, Button::B.mask()), 0x33);
        assert_eq!(read_pad(0x40, 0x40, Button::C.mask()), 0x5f);
        assert_eq!(read_pad(0, 0x40, Button::A.mask()), 0x23);
    }

    #[test]
    fn output_pins_override_buttons_and_th_floats_high_as_an_input() {
        assert_eq!(read_pad(0x81, 0x01, Button::Up.mask()), 0xff);
        assert_eq!(read_pad(0, 0x01, 0), 0x7e);
        assert_eq!(read_pad(0x40, 0x40, 0xff), 0x40);
        assert_eq!(read_pad(0, 0x40, 0xff), 0);
    }
}
