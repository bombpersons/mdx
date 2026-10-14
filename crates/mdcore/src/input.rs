//! Mega Drive controller pins (3- and 6-button pads, GPGX gamepad.c). Pressed
//! buttons are active-low.
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

/// One controller port's pad logic: TH input state, the 6-button TH pulse
/// counter (reset 27 line starts after its last counted rising edge) and the
/// 172-clock TH 0->1 switching latency when TH becomes an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct PadPort {
    state: u8,
    counter: u8,
    /// Absolute line number of the last counted TH rising edge.
    edge_line: u64,
    /// Absolute master clock before which TH still reads low.
    latency: u64,
}

impl Default for PadPort {
    fn default() -> Self {
        Self { state: 0x40, counter: 0, edge_line: 0, latency: 0 }
    }
}

/// Lines between a counted edge and the reference's counter reset.
const SIX_BUTTON_TIMEOUT_LINES: u64 = 27;

impl PadPort {
    fn counter(&self, line: u64) -> u8 {
        if line >= self.edge_line + SIX_BUTTON_TIMEOUT_LINES { 0 } else { self.counter }
    }

    /// GPGX gamepad_write: `data` is the port data register, `mask` its control.
    pub(crate) fn write(&mut self, data: u8, mask: u8, six: bool, now: u64, line: u64) {
        let data = if mask & 0x40 != 0 {
            let data = data & 0x40;
            self.latency = 0;
            let counter = self.counter(line);
            if six && counter < 8 {
                if data != 0 && self.state == 0 {
                    self.counter = counter + 2;
                    self.edge_line = line;
                } else {
                    self.counter = counter;
                }
            }
            data
        } else {
            if self.state == 0 {
                self.latency = now + 172;
            }
            0x40
        };
        self.state = data;
    }

    /// GPGX gamepad_read: input pin levels (D6 = TH, D5-D0 buttons).
    pub(crate) fn read(&self, pressed: u8, now: u64, line: u64) -> u8 {
        let mut data = self.state | 0x3F;
        let pad = gpgx_buttons(pressed);
        let mut step = self.counter(line) | (data >> 6);
        if now < self.latency {
            step &= !1;
        }
        let clear = match step {
            4 => ((pad >> 2) & 0x30) | 0x0F,
            7 => (pad & 0x30) | ((pad >> 8) & 0x0F),
            6 => (pad >> 2) & 0x30,
            _ if step & 1 != 0 => pad & 0x3F,
            _ => (pad & 0x03) | ((pad >> 2) & 0x30) | 0x0C,
        };
        data &= !(clear as u8);
        data
    }
}

/// mdx button mask to the reference's pad bits (U D L R B C A S, then Z Y X M).
fn gpgx_buttons(pressed: u8) -> u16 {
    let b = |button: Button, bit: u16| if pressed & button.mask() != 0 { bit } else { 0 };
    b(Button::Up, 0x01) | b(Button::Down, 0x02) | b(Button::Left, 0x04) | b(Button::Right, 0x08)
        | b(Button::B, 0x10) | b(Button::C, 0x20) | b(Button::A, 0x40) | b(Button::Start, 0x80)
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
    fn six_button_pad_identifies_on_the_third_low_and_times_out() {
        let mut pad = PadPort::default();
        let mut reads = Vec::new();
        for _ in 0..4 {
            pad.write(0x40, 0x40, true, 0, 10);
            reads.push(pad.read(Button::Start.mask(), 0, 10));
            pad.write(0x00, 0x40, true, 0, 10);
            reads.push(pad.read(Button::Start.mask(), 0, 10));
        }
        assert_eq!(reads, [0x7f, 0x13, 0x7f, 0x13, 0x7f, 0x10, 0x7f, 0x1f]);
        // 27 line starts after the last counted edge the counter is reset.
        assert_eq!(pad.read(0, 0, 10 + 26), 0x3f & 0x3f);
        pad.write(0x40, 0x40, true, 0, 40);
        pad.write(0x00, 0x40, true, 0, 40);
        assert_eq!(pad.read(0, 0, 40), 0x33);
    }

    #[test]
    fn th_released_to_input_reads_low_for_the_switching_latency() {
        let mut pad = PadPort::default();
        pad.write(0x00, 0x40, false, 1000, 0);
        pad.write(0x00, 0x00, false, 1000, 0);
        // TH already reads high on D6, but the pad still drives its TH-low
        // layout (D3-D2 low) until the switch completes.
        assert_eq!(pad.read(0, 1100, 0), 0x73);
        assert_eq!(pad.read(0, 1172, 0), 0x7f);
    }

    #[test]
    fn output_pins_override_buttons_and_th_floats_high_as_an_input() {
        assert_eq!(read_pad(0x81, 0x01, Button::Up.mask()), 0xff);
        assert_eq!(read_pad(0, 0x01, 0), 0x7e);
        assert_eq!(read_pad(0x40, 0x40, 0xff), 0x40);
        assert_eq!(read_pad(0, 0x40, 0xff), 0);
    }
}
