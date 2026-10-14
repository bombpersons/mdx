//! Per-console YM2612/PSG synthesis with integer stereo resampling.
//! FM status/timers visible to CPUs remain the separately validated Fm model.
use crate::psg::Psg;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc};
use ymfm_sys::{
    ffi::{self, ChipType},
    ChipPtr, InterfaceCallbacks, InterfaceHandler,
};

pub const SAMPLE_RATE: u64 = 44_100;
/// NTSC master clock.
pub const MASTER_HZ: u64 = 53_693_175;
/// PAL master clock.
pub const PAL_MASTER_HZ: u64 = 53_203_424;

fn ntsc_master_hz() -> u64 {
    MASTER_HZ
}
const FM_SAMPLE_CLOCKS: u64 = 7 * 144;
const FM_CLOCK_HZ: u32 = 7_670_454;
const BACKEND: &str = "ymfm-sys-0.2.0/ym2612-7670454/native144";

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Host {
    clock: u64,
    timers: [Option<u64>; 2],
    busy_until: u64,
    irq: bool,
}

struct FmSynth {
    chip: RefCell<ChipPtr>,
    host: Rc<RefCell<Host>>,
    address: u16,
}

impl FmSynth {
    fn new() -> Self {
        let host = Rc::new(RefCell::new(Host::default()));
        let time = host.clone();
        let timer = host.clone();
        let busy = host.clone();
        let set_busy = host.clone();
        let irq = host.clone();
        let handlers = InterfaceHandler {
            advance_clock: Some(Box::new(move |clocks| {
                let mut h = time.borrow_mut();
                h.clock += clocks.max(0) as u64;
                let mut expired = 0;
                for i in 0..2 {
                    if h.timers[i].is_some_and(|at| at <= h.clock) {
                        h.timers[i] = None;
                        expired |= 1 << i;
                    }
                }
                expired
            })),
            ymfm_set_timer: Some(Box::new(move |number, duration| {
                let mut h = timer.borrow_mut();
                let now = h.clock;
                if let Some(slot) = h.timers.get_mut(number as usize) {
                    *slot = (duration >= 0).then(|| now + duration as u64);
                }
            })),
            ymfm_is_busy: Some(Box::new(move || {
                let h = busy.borrow();
                h.clock < h.busy_until
            })),
            ymfm_set_busy_end: Some(Box::new(move |clocks| {
                let mut h = set_busy.borrow_mut();
                h.busy_until = h.clock + u64::from(clocks);
            })),
            ymfm_update_irq: Some(Box::new(move |asserted| irq.borrow_mut().irq = asserted)),
            ..Default::default()
        };
        let chip = ffi::create_chip_with_callbacks(
            ChipType::Ym2612,
            FM_CLOCK_HZ,
            Box::new(InterfaceCallbacks::new(handlers)),
        );
        assert_eq!(chip.channels(), 2);
        assert_eq!(chip.sample_rate(), 53_267);
        Self {
            chip: RefCell::new(chip),
            host,
            address: 0,
        }
    }
    fn write(&mut self, port: u8, value: u8) {
        if port & 1 == 0 {
            self.address = u16::from(value) | if port == 2 { 256 } else { 0 };
        }
        // The modeled console has one bank-tagged address latch; both data
        // ports consume it. Normalize the backend's bank-specific data entry.
        let port = if port & 1 != 0 {
            if self.address & 256 != 0 {
                3
            } else {
                1
            }
        } else {
            port
        };
        self.chip
            .borrow_mut()
            .pin_mut()
            .write(u32::from(port), value);
    }
    fn sample(&mut self) -> [i32; 2] {
        let mut sample = [0; 2];
        self.chip.borrow_mut().pin_mut().generate(&mut sample);
        sample
    }
    fn state(&self) -> FmState {
        FmState {
            backend: BACKEND.into(),
            data: self.chip.borrow_mut().pin_mut().save_state(),
            host: self.host.borrow().clone(),
            address: self.address,
        }
    }
    fn restore(s: FmState) -> Result<Self, String> {
        // Pinned upstream save_restore layout: 5 wrapper bytes, 11 engine
        // bytes, 517 register/LFO bytes, six 6-byte channels, 24 10-byte operators.
        // Validate before handing external bytes to the native backend.
        let u16_at = |at| u16::from_le_bytes([s.data[at], s.data[at + 1]]);
        if s.backend != BACKEND || s.data.len() != 809 {
            return Err("incompatible FM synthesis backend/state size".into());
        }
        if s.address > 511
            || u16_at(0) != s.address
            || u16_at(2) > 511
            || s.data[4] > 1
            || s.data[10] != 6
            || s.data[11] & !3 != 0
            || s.data[12] > 1
            || s.data[13] > 1
            || s.data[14] > 1
            || s.data[20] > 63
            || (0..24).any(|n| {
                let at = 569 + n * 10;
                u16_at(at + 4) > 1023
                    || !(1..=4).contains(&s.data[at + 6])
                    || s.data[at + 7] > 1
                    || s.data[at + 8] > 1
                    || s.data[at + 9] & !7 != 0
            })
            || s.host.clock > u64::MAX / 4096
            || s.host.busy_until > u64::MAX / 4096
            || s.host
                .timers
                .iter()
                .flatten()
                .any(|at| *at > s.host.clock.saturating_add(589_824))
        {
            return Err("invalid FM synthesis register, envelope or host phase".into());
        }
        let mut chip = Self::new();
        chip.chip.borrow_mut().pin_mut().restore_state(&s.data);
        *chip.host.borrow_mut() = s.host;
        chip.address = s.address;
        Ok(chip)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FmState {
    backend: String,
    data: Vec<u8>,
    host: Host,
    address: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    FmWrite { port: u8, value: u8 },
    PsgWrite { value: u8 },
    FmReset,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    master: u64,
    kind: EventKind,
}

/// Last value written to every YM2612 register (ymfm has no readback), so a
/// register log can begin with the chip's current state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FmShadow {
    /// 0x000..0x0FF bank 0, 0x100..0x1FF bank 1.
    registers: Vec<u8>,
    /// Shared bank-tagged address latch, as `FmSynth` models it.
    address: u16,
    /// Last key on/off (register 0x28) value per channel code 0..7.
    keys: [u8; 8],
}

impl Default for FmShadow {
    fn default() -> Self {
        Self { registers: vec![0; 512], address: 0, keys: [0; 8] }
    }
}

impl FmShadow {
    /// Track a port write; returns (bank, register, value) for data writes.
    fn write(&mut self, port: u8, value: u8) -> Option<(u8, u8, u8)> {
        if port & 1 == 0 {
            self.address = u16::from(value) | if port == 2 { 256 } else { 0 };
            return None;
        }
        self.registers[usize::from(self.address)] = value;
        if self.address == 0x28 {
            self.keys[usize::from(value & 7)] = value;
        }
        Some(((self.address >> 8) as u8, self.address as u8, value))
    }

    /// Writes that recreate the shadowed state: global registers, operator and
    /// channel registers (frequency high bytes before low), then key states.
    fn restore_writes(&self) -> Vec<(u8, u8, u8)> {
        let mut out = Vec::new();
        let reg = |bank: u8, r: u8| self.registers[usize::from(bank) * 256 + usize::from(r)];
        out.push((0, 0x22, reg(0, 0x22)));
        out.push((0, 0x27, reg(0, 0x27) & 0xC0));
        out.push((0, 0x2B, reg(0, 0x2B)));
        out.push((0, 0x2A, reg(0, 0x2A)));
        for bank in 0..2u8 {
            for r in 0x30..=0x9Fu8 {
                if r & 3 != 3 {
                    out.push((bank, r, reg(bank, r)));
                }
            }
            for r in [0xA4, 0xA5, 0xA6, 0xA0, 0xA1, 0xA2, 0xAC, 0xAD, 0xAE, 0xA8, 0xA9, 0xAA] {
                if bank == 0 || r < 0xA8 {
                    out.push((bank, r, reg(bank, r)));
                }
            }
            for r in [0xB0, 0xB1, 0xB2, 0xB4, 0xB5, 0xB6] {
                out.push((bank, r, reg(bank, r)));
            }
        }
        for code in [0u8, 1, 2, 4, 5, 6] {
            out.push((0, 0x28, (self.keys[usize::from(code)] & 0xF0) | code));
        }
        out
    }
}

/// One chip register write in a register log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterWrite {
    Fm { bank: u8, register: u8, value: u8 },
    Psg { value: u8 },
    /// The YM2612 was reset (console reset); not expressible in VGM.
    FmReset,
}

/// Opt-in, bounded log of FM/PSG register writes in synthesis order.
pub struct RegisterLog {
    /// Audio master clock at which the log (and its initial state) begins.
    pub start_master: u64,
    /// Writes recreating the chips' state at `start_master`.
    pub initial: Vec<RegisterWrite>,
    /// False when the FM shadow came from a state saved before shadows existed.
    pub fm_initial_known: bool,
    pub writes: Vec<(u64, RegisterWrite)>,
    pub max_writes: usize,
    pub omitted: u64,
    /// FM resets seen during the log; a VGM cannot express them.
    pub fm_resets: u64,
    /// Complete synthesis state at `start_master` (pending events removed,
    /// since the log records them), for exact re-synthesis.
    pub start_state: AudioState,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioState {
    /// Master clock rate driving sample conversion (NTSC when absent).
    #[serde(default = "ntsc_master_hz")]
    pub master_hz: u64,
    /// Absent in states saved before register shadows; restored as unknown.
    #[serde(default)]
    fm_shadow: Option<FmShadow>,
    fm: FmState,
    psg: Psg,
    clock: u64,
    next_fm: u64,
    fm_sample: [i32; 2],
    psg_level: i32,
    remaining: u64,
    area: [i64; 2],
    previous_input: [i64; 2],
    previous_output: [i64; 2],
    generated: u64,
    events: Vec<Event>,
}

pub struct Audio {
    master_hz: u64,
    fm: FmSynth,
    psg: Psg,
    clock: u64,
    next_fm: u64,
    fm_sample: [i32; 2],
    psg_level: i32,
    remaining: u64,
    area: [i64; 2],
    previous_input: [i64; 2],
    previous_output: [i64; 2],
    generated: u64,
    events: Vec<Event>,
    capture: bool,
    pcm: Vec<i16>,
    capture_limit: usize,
    pub dropped_frames: u64,
    pub clipped_samples: u64,
    fm_shadow: FmShadow,
    fm_shadow_known: bool,
    pub register_log: Option<RegisterLog>,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}
impl Audio {
    pub fn new() -> Self {
        Self {
            fm: FmSynth::new(),
            psg: Psg::new(),
            clock: 0,
            next_fm: FM_SAMPLE_CLOCKS,
            fm_sample: [0; 2],
            psg_level: 0,
            remaining: MASTER_HZ,
            master_hz: MASTER_HZ,
            area: [0; 2],
            previous_input: [0; 2],
            previous_output: [0; 2],
            generated: 0,
            events: Vec::new(),
            capture: false,
            pcm: Vec::new(),
            capture_limit: 0,
            dropped_frames: 0,
            clipped_samples: 0,
            fm_shadow: FmShadow::default(),
            fm_shadow_known: true,
            register_log: None,
        }
    }
    /// Start logging register writes from the current synthesis clock, with
    /// initial writes that recreate the present FM and PSG state.
    pub fn start_register_log(&mut self, max_writes: usize) {
        let mut initial: Vec<RegisterWrite> = self
            .fm_shadow
            .restore_writes()
            .into_iter()
            .map(|(bank, register, value)| RegisterWrite::Fm { bank, register, value })
            .collect();
        for (index, value) in self.psg.registers().into_iter().enumerate() {
            let latch = 0x80 | (index as u8) << 4 | (value & 15) as u8;
            initial.push(RegisterWrite::Psg { value: latch });
            if index % 2 == 0 && index < 6 {
                initial.push(RegisterWrite::Psg { value: (value >> 4) as u8 & 0x3F });
            }
        }
        let mut start_state = self.save_state();
        start_state.events.clear();
        self.register_log = Some(RegisterLog {
            start_master: self.clock,
            initial,
            fm_initial_known: self.fm_shadow_known,
            writes: Vec::new(),
            max_writes,
            omitted: 0,
            fm_resets: 0,
            start_state,
        });
    }
    fn log_write(&mut self, master: u64, write: RegisterWrite) {
        if let Some(log) = self.register_log.as_mut() {
            if log.writes.len() < log.max_writes {
                log.writes.push((master, write));
            } else {
                log.omitted += 1;
            }
        }
    }
    /// Select the PAL or NTSC master clock; only before audio has advanced.
    pub fn set_pal(&mut self, pal: bool) {
        self.master_hz = if pal { PAL_MASTER_HZ } else { MASTER_HZ };
        self.remaining = self.master_hz;
    }

    pub fn master_hz(&self) -> u64 {
        self.master_hz
    }

    pub fn clock(&self) -> u64 {
        self.clock
    }
    pub fn generated_frames(&self) -> u64 {
        self.generated
    }
    pub fn start_capture(&mut self, max_frames: usize) {
        self.capture = true;
        self.pcm.clear();
        self.capture_limit = max_frames;
        self.dropped_frames = 0;
        self.clipped_samples = 0;
    }
    pub fn take_pcm(&mut self) -> Vec<i16> {
        self.capture = false;
        std::mem::take(&mut self.pcm)
    }
    pub fn enqueue(&mut self, master: u64, kind: EventKind) {
        // CPU/sound batches may be observed in different clock order; sort at
        // the safe render horizon and retain stable ordering of equal clocks.
        assert!(
            master >= self.clock,
            "sound write predates completed synthesis"
        );
        self.events.push(Event { master, kind });
    }
    pub fn advance_to(&mut self, target: u64) {
        assert!(target >= self.clock);
        self.events.sort_by_key(|e| e.master);
        let consumed = self.events.partition_point(|e| e.master <= target);
        let events: Vec<_> = self.events.drain(..consumed).collect();
        for event in events {
            self.render_until(event.master);
            match event.kind {
                EventKind::FmWrite { port, value } => {
                    self.fm.write(port, value);
                    if let Some((bank, register, value)) = self.fm_shadow.write(port, value) {
                        self.log_write(event.master, RegisterWrite::Fm { bank, register, value });
                    }
                }
                EventKind::PsgWrite { value } => {
                    self.log_write(event.master, RegisterWrite::Psg { value });
                    let edges = self.psg.write(event.master, value);
                    debug_assert!(edges.is_empty());
                }
                EventKind::FmReset => {
                    self.fm_shadow = FmShadow::default();
                    if let Some(log) = self.register_log.as_mut() {
                        log.fm_resets += 1;
                    }
                    self.log_write(event.master, RegisterWrite::FmReset);
                    self.fm = FmSynth::new();
                    self.fm_sample = [0; 2];
                    self.next_fm = event.master + FM_SAMPLE_CLOCKS;
                }
            }
        }
        self.render_until(target);
    }
    fn render_until(&mut self, target: u64) {
        let edges = self.psg.advance_to(target);
        let mut index = 0;
        while self.clock < target {
            let next = target
                .min(self.next_fm)
                .min(edges.get(index).map_or(u64::MAX, |e| e.master));
            self.integrate(next - self.clock);
            self.clock = next;
            while edges.get(index).is_some_and(|e| e.master == next) {
                self.psg_level += edges[index].delta;
                index += 1;
            }
            if self.next_fm == next {
                self.fm_sample = self.fm.sample();
                self.next_fm += FM_SAMPLE_CLOCKS;
            }
        }
    }
    fn integrate(&mut self, masters: u64) {
        let mut units = masters * SAMPLE_RATE;
        while units != 0 {
            let span = units.min(self.remaining);
            units -= span;
            self.remaining -= span;
            for ch in 0..2 {
                self.area[ch] +=
                    (i64::from(self.fm_sample[ch]) + i64::from(self.psg_level)) * span as i64;
            }
            if self.remaining == 0 {
                self.generated += 1;
                let mut output = [0i16; 2];
                for (ch, sample) in output.iter_mut().enumerate() {
                    let input = self.area[ch] / self.master_hz as i64;
                    let highpass =
                        input - self.previous_input[ch] + self.previous_output[ch] * 995 / 1000;
                    self.previous_input[ch] = input;
                    self.previous_output[ch] = highpass;
                    if self.capture && !(-32768..=32767).contains(&highpass) {
                        self.clipped_samples += 1;
                    }
                    *sample = highpass.clamp(-32768, 32767) as i16;
                }
                if self.capture {
                    if self.pcm.len() / 2 < self.capture_limit {
                        self.pcm.extend(output);
                    } else {
                        self.dropped_frames += 1;
                    }
                }
                self.area = [0; 2];
                self.remaining = self.master_hz;
            }
        }
    }
    pub fn save_state(&self) -> AudioState {
        AudioState {
            master_hz: self.master_hz,
            fm_shadow: self.fm_shadow_known.then(|| self.fm_shadow.clone()),
            fm: self.fm.state(),
            psg: self.psg.clone(),
            clock: self.clock,
            next_fm: self.next_fm,
            fm_sample: self.fm_sample,
            psg_level: self.psg_level,
            remaining: self.remaining,
            area: self.area,
            previous_input: self.previous_input,
            previous_output: self.previous_output,
            generated: self.generated,
            events: self.events.clone(),
        }
    }
    /// Rebuild from a state captured in this process (no validation).
    fn resume(s: AudioState) -> Self {
        let fm_shadow_known = s.fm_shadow.is_some();
        Self {
            master_hz: s.master_hz,
            fm: FmSynth::restore(s.fm).expect("state captured by this process"),
            psg: s.psg,
            clock: s.clock,
            next_fm: s.next_fm,
            fm_sample: s.fm_sample,
            psg_level: s.psg_level,
            remaining: s.remaining,
            area: s.area,
            previous_input: s.previous_input,
            previous_output: s.previous_output,
            generated: s.generated,
            events: s.events,
            capture: false,
            pcm: Vec::new(),
            capture_limit: 0,
            dropped_frames: 0,
            clipped_samples: 0,
            fm_shadow: s.fm_shadow.unwrap_or_default(),
            fm_shadow_known,
            register_log: None,
        }
    }

    pub fn from_state(s: AudioState, console_master: u64) -> Result<Self, String> {
        let scaled = u128::from(s.clock) * u128::from(SAMPLE_RATE);
        if s.clock > u64::MAX / 4096
            || s.psg.master_clock() != s.clock.div_ceil(240) * 240
            || s.clock > console_master
            || console_master - s.clock > 3420 + 100_000_000
            || s.next_fm < s.clock
            || s.next_fm > s.clock.saturating_add(FM_SAMPLE_CLOCKS)
            || !matches!(s.master_hz, MASTER_HZ | PAL_MASTER_HZ)
            || s.generated != (scaled / u128::from(s.master_hz)) as u64
            || s.remaining != s.master_hz - (scaled % u128::from(s.master_hz)) as u64
            || s.events.len() > 4096
            || s.fm_shadow.as_ref().is_some_and(|shadow| shadow.registers.len() != 512 || shadow.address > 511)
            || !(0..=11200).contains(&s.psg_level)
            || s.fm_sample.iter().any(|n| i64::from(*n).abs() > 1_048_576)
            || s.area
                .iter()
                .any(|n| n.unsigned_abs() > 1_100_000 * s.master_hz)
            || s.previous_input
                .iter()
                .chain(&s.previous_output)
                .any(|n| n.unsigned_abs() > 8_388_608)
            || s.events.iter().any(|e| {
                e.master < s.clock
                    || e.master > console_master.saturating_add(100_000_000)
                    || matches!(e.kind,EventKind::FmWrite {port,..} if port>3)
            })
        {
            return Err("invalid synthesis/mixer clock, buffer or event state".into());
        }
        let fm = FmSynth::restore(s.fm)?;
        let fm_shadow_known = s.fm_shadow.is_some();
        Ok(Self {
            master_hz: s.master_hz,
            fm_shadow: s.fm_shadow.unwrap_or_default(),
            fm_shadow_known,
            register_log: None,
            fm,
            psg: s.psg,
            clock: s.clock,
            next_fm: s.next_fm,
            fm_sample: s.fm_sample,
            psg_level: s.psg_level,
            remaining: s.remaining,
            area: s.area,
            previous_input: s.previous_input,
            previous_output: s.previous_output,
            generated: s.generated,
            events: s.events,
            capture: false,
            pcm: Vec::new(),
            capture_limit: 0,
            dropped_frames: 0,
            clipped_samples: 0,
        })
    }
}

/// Voices that `render_voices` can isolate: FM channels 0..5 then PSG 0..3.
pub const VOICES: [&str; 10] = ["fm1", "fm2", "fm3", "fm4", "fm5", "fm6", "psg1", "psg2", "psg3", "psg4"];

/// Re-synthesize a register log from its exact start state, keeping only
/// voices whose bit is set in `keep` (bit n = `VOICES[n]`). Other FM channels
/// get maximum total-level attenuation and key-offs, from the start and on
/// every later write; the DAC (which replaces FM6) is disabled unless FM6 is
/// kept; other PSG channels are muted in the start state and held at
/// attenuation 15. With every voice kept
/// the result equals the original capture. Every render starts from the same
/// exact mixer state, so a transient the output filter carries into the
/// capture (e.g. at power-on) appears in each stem. Returns interleaved stereo PCM.
pub fn render_voices(log: &RegisterLog, end_master: u64, keep: u16) -> Vec<i16> {
    let fm_kept = |channel: u8| keep & (1 << channel) != 0;
    let psg_kept = |channel: u8| keep & (1 << (6 + channel)) != 0;
    // The game's PSG writes continue from the start state's latch.
    let mut psg_latch = log.start_state.psg.latch();
    let mut filter = |write: RegisterWrite| -> RegisterWrite {
        match write {
            RegisterWrite::Fm { bank: 0, register: 0x28, value } => {
                let code = value & 7;
                let channel = if code >= 4 { code - 1 } else { code };
                if code == 3 || code == 7 || fm_kept(channel) {
                    write
                } else {
                    RegisterWrite::Fm { bank: 0, register: 0x28, value: value & 0x0F }
                }
            }
            RegisterWrite::Fm { bank, register: register @ 0x40..=0x4F, .. }
                if register & 3 != 3 && !fm_kept(bank * 3 + (register & 3)) =>
            {
                RegisterWrite::Fm { bank, register, value: 0x7F }
            }
            RegisterWrite::Fm { bank: 0, register: 0x2B, .. } if !fm_kept(5) => {
                RegisterWrite::Fm { bank: 0, register: 0x2B, value: 0 }
            }
            RegisterWrite::Psg { value } => {
                if value & 0x80 != 0 {
                    psg_latch = (value >> 4) & 7;
                }
                if psg_latch & 1 != 0 && !psg_kept(psg_latch >> 1) {
                    RegisterWrite::Psg { value: value | 0x0F }
                } else {
                    write
                }
            }
            other => other,
        }
    };
    let mut silence = Vec::new();
    for channel in 0..6u8 {
        if !fm_kept(channel) {
            let (bank, slot) = (channel / 3, channel % 3);
            for operator in 0..4u8 {
                silence.push(RegisterWrite::Fm { bank, register: 0x40 + operator * 4 + slot, value: 0x7F });
            }
            let code = if channel >= 3 { channel + 1 } else { channel };
            silence.push(RegisterWrite::Fm { bank: 0, register: 0x28, value: code });
        }
    }
    if !fm_kept(5) {
        silence.push(RegisterWrite::Fm { bank: 0, register: 0x2B, value: 0 });
    }
    let mut audio = Audio::resume(log.start_state.clone());
    // An isolated voice cannot inherit the mix's output-filter history, so
    // it starts with the filter at rest on the kept voices' current level:
    // continuous (no step) and silent where nothing is kept. Every voice kept
    // leaves the exact state untouched.
    if keep & 0x3FF != 0x3FF {
        for channel in 0..4u8 {
            if !psg_kept(channel) {
                let removed = audio.psg.mute(usize::from(channel));
                audio.psg_level -= removed;
            }
        }
        if keep & 0x3F != 0x3F {
            // Hold the kept voices' next FM sample from the start: the chip
            // with the silencing writes applied, probed on a copy.
            let mut chip = FmSynth::restore(log.start_state.fm.clone()).expect("captured state");
            for write in &silence {
                if let RegisterWrite::Fm { bank, register, value } = *write {
                    chip.write(bank * 2, register);
                    chip.write(bank * 2 + 1, value);
                }
            }
            audio.fm_sample = chip.sample();
        }
        let integrated = (audio.master_hz - audio.remaining) as i64;
        for side in 0..2 {
            let level = i64::from(audio.fm_sample[side]) + i64::from(audio.psg_level);
            audio.previous_input[side] = level;
            audio.previous_output[side] = 0;
            audio.area[side] = level * integrated;
        }
    }
    let span = end_master.saturating_sub(log.start_master);
    let frames = (u128::from(span) * u128::from(SAMPLE_RATE) / u128::from(audio.master_hz)) as usize;
    audio.start_capture(frames + 2);
    let start = log.start_master;
    let push = |audio: &mut Audio, master: u64, write: RegisterWrite| match write {
        RegisterWrite::Fm { bank, register, value } => {
            audio.enqueue(master, EventKind::FmWrite { port: bank * 2, value: register });
            audio.enqueue(master, EventKind::FmWrite { port: bank * 2 + 1, value });
        }
        RegisterWrite::Psg { value } => audio.enqueue(master, EventKind::PsgWrite { value }),
        RegisterWrite::FmReset => audio.enqueue(master, EventKind::FmReset),
    };
    for write in silence {
        push(&mut audio, start, write);
    }
    for (master, write) in &log.writes {
        let write = filter(*write);
        push(&mut audio, (*master).max(start), write);
    }
    audio.advance_to(end_master.max(start));
    audio.take_pcm()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(audio: &mut Audio, master: u64, pan: u8) {
        let mut register = |address, value| {
            audio.enqueue(
                master,
                EventKind::FmWrite {
                    port: 0,
                    value: address,
                },
            );
            audio.enqueue(master, EventKind::FmWrite { port: 1, value });
        };
        for slot in 0..4 {
            register(0x30 + slot * 4, 1);
            register(0x40 + slot * 4, 0);
            register(0x50 + slot * 4, 0x1f);
            register(0x80 + slot * 4, 0x0f);
        }
        register(0xb0, 7);
        register(0xb4, pan);
        register(0xa4, 0x22);
        register(0xa0, 0x69);
        register(0x28, 0xf0);
    }
    #[test]
    fn fm_tone_obeys_stereo_panning_and_psg_is_audible() {
        let mut audio = Audio::new();
        tone(&mut audio, 0, 0x80);
        audio.start_capture(4410);
        audio.advance_to(MASTER_HZ / 10);
        let pcm = audio.take_pcm();
        assert_eq!(
            pcm.len(),
            ((MASTER_HZ / 10 * SAMPLE_RATE / MASTER_HZ) * 2) as usize
        );
        assert!(pcm.chunks_exact(2).any(|s| s[0] != 0));
        // The YM2612 backend includes a ladder DAC DC offset even on a
        // muted side. Compare with an unprogrammed chip rather than assuming
        // its high-pass startup transient is zero.
        let mut idle = Audio::new();
        idle.start_capture(4410);
        idle.advance_to(MASTER_HZ / 10);
        let silence = idle.take_pcm();
        assert!(pcm
            .chunks_exact(2)
            .zip(silence.chunks_exact(2))
            .all(|(a, b)| a[1] == b[1]));
        assert!(pcm
            .chunks_exact(2)
            .zip(silence.chunks_exact(2))
            .any(|(a, b)| a[0] != b[0]));
        let mut psg = Audio::new();
        for value in [0x80, 0x04, 0x90] {
            psg.enqueue(0, EventKind::PsgWrite { value });
        }
        psg.start_capture(4410);
        psg.advance_to(MASTER_HZ / 10);
        let pcm = psg.take_pcm();
        assert!(pcm.iter().any(|s| *s != 0));
        assert!(pcm.chunks_exact(2).all(|s| s[0] == s[1]));
    }
    #[test]
    fn checkpoint_retains_pending_writes_timers_and_fractional_sample_phase() {
        let mut live = Audio::new();
        tone(&mut live, 0, 0xc0);
        for (port, value) in [
            (0, 0x24),
            (1, 0xff),
            (0, 0x25),
            (1, 3),
            (0, 0x27),
            (1, 0x85),
        ] {
            live.enqueue(0, EventKind::FmWrite { port, value });
        }
        live.enqueue(102_003, EventKind::PsgWrite { value: 0x90 });
        live.enqueue(210_005, EventKind::FmReset);
        tone(&mut live, 220_003, 0x40);
        live.advance_to(91_327);
        let saved = live.save_state();
        assert!(saved.fm.host.irq);
        let bytes = serde_json::to_vec(&saved).unwrap();
        let state = serde_json::from_slice(&bytes).unwrap();
        let mut restored = Audio::from_state(state, 91_327).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&restored.save_state()).unwrap());
        live.start_capture(10000);
        restored.start_capture(10000);
        live.advance_to(1_000_000);
        for until in [100_000, 200_001, 210_005, 300_007, 1_000_000] {
            restored.advance_to(until);
        }
        assert_eq!(live.take_pcm(), restored.take_pcm());
        assert_eq!(
            serde_json::to_vec(&live.save_state()).unwrap(),
            serde_json::to_vec(&restored.save_state()).unwrap()
        );
    }
    #[test]
    fn capture_limits_never_change_synthesis_state_and_bad_native_state_is_rejected() {
        let mut captured = Audio::new();
        let mut plain = Audio::new();
        tone(&mut captured, 0, 0xc0);
        tone(&mut plain, 0, 0xc0);
        captured.start_capture(1);
        captured.advance_to(100_000);
        plain.advance_to(100_000);
        assert!(captured.dropped_frames > 0);
        assert_eq!(captured.take_pcm().len(), 2);
        let state = plain.save_state();
        assert_eq!(
            serde_json::to_vec(&captured.save_state()).unwrap(),
            serde_json::to_vec(&state).unwrap()
        );
        let mut bad = state.clone();
        bad.fm.data[575] = 255;
        assert!(Audio::from_state(bad, 100_000).is_err());
        let mut bad = state;
        bad.remaining = 0;
        assert!(Audio::from_state(bad, 100_000).is_err());
        let mut overflow = Audio::new().save_state();
        overflow.clock = u64::MAX;
        assert!(Audio::from_state(overflow, u64::MAX).is_err());
        assert!(Audio::from_state(Audio::new().save_state(), 0).is_ok());
    }
}
