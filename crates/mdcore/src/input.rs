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

/// Light guns the reference attaches to port B for known titles (GPGX
/// loadrom.c autodetection, input_hw/lightgun.c). Nothing pulls the trigger,
/// and the libretro frontend aims at the centre of the viewport latched at
/// the previous frame edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum GunKind {
    Menacer,
    Justifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct LightGun {
    pub kind: GunKind,
    state: u8,
    /// Selected Justifier (input device 4 or 5).
    port: u8,
    /// Aim in viewport pixels.
    pub aim: (i32, i32),
    /// Viewport (width, height) latched at the last frame edge.
    viewport: (i32, i32),
    offset: (i32, i32),
}

impl LightGun {
    pub fn detect(international: &[u8]) -> Option<Self> {
        let has = |name: &str| international.windows(name.len()).any(|w| w == name.as_bytes());
        const MENACER: [(&str, i32, i32); 8] = [
            ("MENACER", 82, 0),
            ("T2 ; THE ARCADE GAME", 133, -8),
            ("BODY COUNT", 68, -24),
            ("CORPSE KILLER", 64, -8),
            ("CRIME PATROL", 61, 0),
            ("MAD DOG II THE LOST GOLD", 70, 18),
            ("MAD DOG MCCREE", 49, 0),
            ("WHO SHOT JOHNNY ROCK?", 60, 30),
        ];
        let (kind, offset) = if let Some(&(_, x, y)) = MENACER.iter().find(|(name, _, _)| has(name)) {
            (GunKind::Menacer, (x, y))
        } else if has("LETHAL ENFORCERS") || has("SNATCHER") {
            (GunKind::Justifier, (if has("GUN FIGHTERS") { 24 } else { 0 }, 0))
        } else {
            return None;
        };
        // vdp_reset's 256x192 viewport is what the first frame's aim sees.
        Some(Self { kind, state: 0x40, port: 4, aim: (128, 96), viewport: (256, 192), offset })
    }

    /// Port B input pins.
    pub fn read(&self) -> u8 {
        match self.kind {
            // Buttons active high; TH reads 1 outside the active pixel.
            GunKind::Menacer => 0x40,
            // Gun detection while TH is high; else trigger/start (active low).
            GunKind::Justifier if self.state & 0x40 != 0 => 0x30,
            GunKind::Justifier => 0x73,
        }
    }

    pub fn write(&mut self, data: u8, mask: u8) {
        if self.kind == GunKind::Justifier {
            let data = data & mask;
            self.port = 4 + ((data >> 5) & 1);
            self.state = data;
        }
    }

    /// Frame edge: aim at the centre of the previous viewport, then latch
    /// the new one.
    pub fn frame_edge(&mut self, width: i32, height: i32) {
        self.aim = (self.viewport.0 / 2, self.viewport.1 / 2);
        self.viewport = (width, height);
    }

    /// The gun's line, when `line` is it (and inside the active display),
    /// and the H counter it latches there.
    pub fn hit(&self, line: i32, visible_lines: i32, h40: bool) -> Option<u16> {
        let y = self.aim.1 + self.offset.1;
        if y != line || y >= visible_lines {
            return None;
        }
        let mut x = self.aim.0;
        if self.kind == GunKind::Menacer {
            x = x * 289 / 320;
        }
        let (dots, low_end, high_start) = if h40 { (210, 0xB6, 0xE5) } else { (171, 0x93, 0xE9) };
        let index = ((x / 2) + self.offset.0).rem_euclid(dots);
        let low = low_end + 1;
        let h = if index < low { index } else { high_start + (index - low) };
        Some(((y as u16) << 8) | h as u16)
    }
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

    #[test]
    fn light_guns_follow_the_reference_autodetection_and_counters() {
        assert!(LightGun::detect(b"SOME OTHER GAME").is_none());
        let mut gun = LightGun::detect(b"LETHAL ENFORCERS             ").unwrap();
        assert_eq!(gun.kind, GunKind::Justifier);
        // TH high reports gun detection; TH low the (released) buttons.
        assert_eq!(gun.read(), 0x30);
        gun.write(0x00, 0x40);
        assert_eq!(gun.read(), 0x73);
        // First frame aims at vdp_reset's 256x192 viewport; later frames at
        // the viewport latched one edge earlier.
        assert_eq!(gun.hit(96, 224, false), Some(0x6040));
        gun.frame_edge(320, 224);
        assert_eq!(gun.aim, (128, 96));
        gun.frame_edge(320, 224);
        assert_eq!(gun.aim, (160, 112));
        assert_eq!(gun.hit(111, 224, true), None);
        assert_eq!(gun.hit(112, 224, true), Some(0x7050));
        let menacer = LightGun::detect(b"T2 ; THE ARCADE GAME").unwrap();
        assert_eq!(menacer.kind, GunKind::Menacer);
        assert_eq!(menacer.read(), 0x40);
        // x = 128*289/320 = 115; (57 + 133) mod 171 = 19; y = 96 - 8.
        assert_eq!(menacer.hit(88, 224, false), Some(0x5813));
        // H40: x = 160*289/320 = 144; 72 + 82 = 154.
        let mut far = LightGun::detect(b"MENACER").unwrap();
        far.frame_edge(320, 224);
        far.frame_edge(320, 224);
        assert_eq!(far.hit(112, 224, true), Some(0x709A));
    }
}
