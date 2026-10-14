//! Integrated Mega Drive PSG pulse generator in console master clocks.
//! Emits unfiltered mono level changes; PCM filtering/mixing belongs to the host.
//! System routes timed writes through the per-console audio mixer.
use serde::{Deserialize, Serialize};

const TICK: u64 = 15 * 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub master: u64,
    pub delta: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "PsgState")]
pub struct Psg {
    clock: u64,
    latch: u8,
    registers: [u16; 8],
    periods: [u64; 4],
    next: [u64; 4],
    high: [bool; 4],
    noise: u16,
    amplitude: [i32; 4],
    pending: [i32; 4],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PsgState {
    clock: u64,
    latch: u8,
    registers: [u16; 8],
    periods: [u64; 4],
    next: [u64; 4],
    high: [bool; 4],
    noise: u16,
    amplitude: [i32; 4],
    pending: [i32; 4],
}

impl TryFrom<PsgState> for Psg {
    type Error = &'static str;
    fn try_from(s: PsgState) -> Result<Self, Self::Error> {
        let tone_periods =
            [s.registers[0], s.registers[2], s.registers[4]].map(|n| u64::from(n.max(1)) * TICK);
        let noise_period = if s.registers[6] & 3 == 3 {
            tone_periods[2]
        } else {
            (16 << (s.registers[6] & 3)) * TICK
        };
        if s.clock > u64::MAX / 4096
            || s.clock % TICK != 0
            || s.latch > 7
            || s.noise == 0
            || [0, 2, 4].iter().any(|i| s.registers[*i] > 1023)
            || [1, 3, 5, 7].iter().any(|i| s.registers[*i] > 15)
            || s.registers[6] > 7
            || s.periods[..3] != tone_periods
            || s.periods[3] != noise_period
            || s.next
                .iter()
                .any(|n| *n < s.clock || *n > s.clock.saturating_add(1023 * TICK) || *n % TICK != 0)
            || s.amplitude.iter().enumerate().any(|(i, a)| {
                *a != Self::level(s.registers[i * 2 + 1])
                    && !(*a == 0 && s.registers[i * 2 + 1] == 0)
            })
            || s.pending.iter().any(|n| !(-2800..=2800).contains(n))
        {
            return Err("invalid integrated PSG register or clock phase");
        }
        Ok(Self {
            clock: s.clock,
            latch: s.latch,
            registers: s.registers,
            periods: s.periods,
            next: s.next,
            high: s.high,
            noise: s.noise,
            amplitude: s.amplitude,
            pending: s.pending,
        })
    }
}

impl Default for Psg {
    fn default() -> Self {
        Self::new()
    }
}

impl Psg {
    /// Integrated-chip reset profile; zero tone period means one internal tick.
    pub fn new() -> Self {
        Self {
            clock: 0,
            latch: 3,
            registers: [0; 8],
            periods: [TICK, TICK, TICK, 16 * TICK],
            next: [0; 4],
            high: [false; 4],
            noise: 0x8000,
            amplitude: [0; 4],
            pending: [0; 4],
        }
    }

    /// Two decibels per attenuation step. Scale is the reference's 100% PSG gain;
    /// these levels are calculated from the attenuation law, not a copied table.
    fn level(attenuation: u16) -> i32 {
        match attenuation {
            0 => 2800,
            10 => 280,
            15 => 0,
            x => (2800.0 * 10.0_f64.powf(-f64::from(x) / 10.0)) as i32,
        }
    }

    /// Advance monotonically, retaining the chip's fractional internal-clock phase.
    pub fn advance_to(&mut self, master: u64) -> Vec<Edge> {
        if master <= self.clock {
            return Vec::new();
        }
        let mut edges = Vec::new();
        for channel in 0..4 {
            if self.pending[channel] != 0 {
                edges.push(Edge {
                    master: self.clock,
                    delta: self.pending[channel],
                });
                self.pending[channel] = 0;
            }
            while self.next[channel] < master {
                self.high[channel] = !self.high[channel];
                let delta = if channel < 3 {
                    if self.high[channel] {
                        self.amplitude[channel]
                    } else {
                        -self.amplitude[channel]
                    }
                } else if self.high[channel] {
                    let before = self.noise & 1;
                    let feedback = if self.registers[6] & 4 != 0 {
                        (self.noise ^ (self.noise >> 3)) & 1
                    } else {
                        before
                    };
                    self.noise = (self.noise >> 1) | (feedback << 15);
                    (i32::from(self.noise & 1) - i32::from(before)) * self.amplitude[3]
                } else {
                    0
                };
                if delta != 0 {
                    edges.push(Edge {
                        master: self.next[channel],
                        delta,
                    });
                }
                self.next[channel] += self.periods[channel];
            }
        }
        self.clock = master.div_ceil(TICK) * TICK;
        edges.sort_by_key(|e| e.master);
        edges
    }

    /// Apply one serial latch/data byte after running to its timestamp.
    /// Silence `channel` (0..3) without a register write (which would move the
    /// latch). Returns the level it contributed to the mixed PSG output.
    pub fn mute(&mut self, channel: usize) -> i32 {
        let on = if channel < 3 { self.high[channel] } else { self.noise & 1 != 0 };
        let contributed = if on { self.amplitude[channel] - self.pending[channel] } else { 0 };
        self.pending[channel] = 0;
        self.amplitude[channel] = 0;
        self.registers[channel * 2 + 1] = 15;
        contributed
    }

    /// Register index (0..7) that a data byte currently writes.
    pub fn latch(&self) -> u8 {
        self.latch
    }

    /// Tone periods / noise mode (even) and attenuations (odd), as written.
    pub fn registers(&self) -> [u16; 8] {
        self.registers
    }

    pub fn write(&mut self, master: u64, data: u8) -> Vec<Edge> {
        let edges = self.advance_to(master);
        if data & 0x80 != 0 {
            self.latch = (data >> 4) & 7;
        }
        let index = usize::from(self.latch);
        let channel = index / 2;
        if index & 1 != 0 {
            let attenuation = u16::from(data & 15);
            let volume = Self::level(attenuation);
            let on = if channel < 3 {
                self.high[channel]
            } else {
                self.noise & 1 != 0
            };
            if on {
                self.pending[channel] += volume - self.amplitude[channel];
            }
            self.amplitude[channel] = volume;
            self.registers[index] = attenuation;
        } else if index == 6 {
            let mode = u16::from(data & 7);
            if mode & 3 == 3 {
                self.periods[3] = self.periods[2];
                self.next[3] = self.next[2];
            } else {
                self.periods[3] = (16 << (mode & 3)) * TICK;
            }
            if self.noise & 1 != 0 {
                self.pending[3] -= self.amplitude[3];
            }
            self.noise = 0x8000;
            self.registers[6] = mode;
        } else {
            let old = self.registers[index];
            let period = if data & 0x80 != 0 {
                (old & 0x3f0) | u16::from(data & 15)
            } else {
                (old & 15) | (u16::from(data & 63) << 4)
            };
            self.registers[index] = period;
            self.periods[channel] = u64::from(period.max(1)) * TICK;
            if channel == 2 && self.registers[6] & 3 == 3 {
                self.periods[3] = self.periods[2];
            }
        }
        edges
    }

    pub(crate) fn master_clock(&self) -> u64 {
        self.clock
    }

    /// Observer projection matching the independent C harness; not a save format.
    pub fn phase(&self) -> serde_json::Value {
        serde_json::json!({"clock":self.clock,"latch":self.latch,"noise":self.noise,
            "periods":self.periods,"next":self.next,
            "polarity":self.high.map(|x|if x {1} else {-1}),"amplitude":self.amplitude})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_restored_periods_cannot_stall_the_pulse_generator() {
        let mut state = serde_json::to_value(Psg::new()).unwrap();
        state["periods"][0] = serde_json::json!(0);
        assert!(serde_json::from_value::<Psg>(state).is_err());
    }
    #[test]
    fn pending_volume_and_noise_phase_survive_serialized_continuation() {
        let mut live = Psg::new();
        live.write(0, 0xc5);
        live.write(0, 0x01);
        live.write(0, 0xe7);
        live.write(0, 0xf0);
        live.advance_to(123457);
        live.write(123460, 0xf3); // includes a delayed level change
        let mut loaded: Psg = serde_json::from_slice(&serde_json::to_vec(&live).unwrap()).unwrap();
        for clock in [123461, 123480, 124001, 136000, 300123] {
            assert_eq!(live.advance_to(clock), loaded.advance_to(clock));
            assert_eq!(live.phase(), loaded.phase());
        }
    }
    #[test]
    fn authored_phase_and_edges_match_the_unchanged_reference_psg() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/psg_edges.json")).unwrap();
        for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
            let mut psg = Psg::new();
            for row in case["operations"].as_array().unwrap() {
                let clock = row["master"].as_u64().unwrap();
                let edges = if let Some(value) = row["write"].as_u64() {
                    psg.write(clock, value as u8)
                } else {
                    psg.advance_to(clock)
                };
                let mut combined = std::collections::BTreeMap::<u64, i32>::new();
                for edge in edges {
                    *combined.entry(edge.master).or_default() += edge.delta;
                }
                let actual: Vec<_> = combined
                    .into_iter()
                    .filter(|(_, delta)| *delta != 0)
                    .map(|(at, delta)| serde_json::json!([at, delta, delta]))
                    .collect();
                assert_eq!(
                    serde_json::json!(actual),
                    row["edges"],
                    "case {index} clock {clock}"
                );
                assert_eq!(psg.phase(), row["phase"], "case {index} clock {clock}");
            }
        }
    }
}
