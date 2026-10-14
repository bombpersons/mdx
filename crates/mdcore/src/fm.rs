//! Register/status model of the Mega Drive's discrete YM2612.
//!
//! Ports 0/2 select one shared, bank-tagged address latch; either data port
//! writes the selected register. Only port 0 returns status on this chip variant.
//! Timing follows Genesis Plus GX's MAME OPN2 wrapper; no audio is synthesized.

/// Two 256-byte register banks. Bank 1 diagnostic indices are 0x100..0x1ff.
const REGISTERS: usize = 512;
const BUSY: u8 = 0x80;
const TIMER_A_OVERFLOW: u8 = 0x01;
const TIMER_B_OVERFLOW: u8 = 0x02;
/// Internal clock: master / 7 (YM input) / 6.
const YM2612_CLOCK_RATIO: u64 = 7 * 6;
/// One MAME OPN2 sample/timer-A count is 24 internal clocks (1008 master).
const SAMPLE_CLOCK_RATIO: u64 = 24 * YM2612_CLOCK_RATIO;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fm {
    #[serde(with = "crate::system::state::array")]
    pub registers: [u8; REGISTERS],
    /// Most recent port-0 address, retained for diagnostics.
    pub port_a: u8,
    /// Most recent port-2 address, retained for diagnostics.
    pub port_b: u8,
    /// Latched timer overflow bits; BUSY is computed when reading status.
    pub status: u8,
    /// Ten-bit timer A reload value, assembled from registers 0x24/0x25.
    pub timer_a: u16,
    pub timer_b: u8,
    /// Legacy address-write diagnostic; not a YM2612 hardware noise source.
    pub lfsr: u16,
    /// Total hardware-port writes, including address writes.
    pub write_count: usize,
    /// Per-register accepted data writes, with banks distinguished.
    #[serde(with = "crate::system::state::array")]
    pub reg_writes: [u32; REGISTERS],
    #[serde(with = "crate::system::state::array")]
    pub reg_last: [u8; REGISTERS],
    /// Writes per hardware port: 0/2 select an address, 1/3 carry data.
    pub port_writes: [u32; 4],
    address: usize,
    /// Chip clock; may run ahead of the system schedule after an access sync.
    master_cycles: u64,
    /// System-driven clock (`tick`); absent in older states, where it equals
    /// the chip clock.
    #[serde(default)]
    scheduled: Option<u64>,
    busy_until: u64,
    timer_control: u8,
    timer_samples: u64,
    timer_a_remaining: u64,
    timer_b_remaining: u64,
}

impl Default for Fm {
    fn default() -> Self {
        Self::new()
    }
}

impl Fm {
    pub(crate) fn valid_state(&self) -> bool {
        self.address < REGISTERS && self.timer_a < 1024
            && self.master_cycles < u64::MAX / 4096
            && self.scheduled.is_none_or(|s| s < u64::MAX / 4096)
            && self.busy_until < u64::MAX / 4096
            && self.timer_samples == self.master_cycles.div_ceil(SAMPLE_CLOCK_RATIO)
            && self.timer_a_remaining <= 1024 && self.timer_b_remaining <= 4096
    }

    pub fn new() -> Self {
        Self {
            registers: [0; REGISTERS],
            port_a: 0,
            port_b: 0,
            status: 0,
            timer_a: 0,
            timer_b: 0,
            lfsr: 1,
            write_count: 0,
            reg_writes: [0; REGISTERS],
            reg_last: [0; REGISTERS],
            port_writes: [0; 4],
            address: 0,
            master_cycles: 0,
            scheduled: None,
            busy_until: 0,
            timer_control: 0,
            timer_samples: 0,
            timer_a_remaining: 0,
            timer_b_remaining: 0,
        }
    }

    /// Reset chip state at the current clock, preserving diagnostic history
    /// and the address latch (GPGX YM2612ResetChip does not clear that latch).
    pub fn reset(&mut self) {
        self.registers.fill(0);
        for bank in [0, 0x100] {
            self.registers[bank + 0xb4..=bank + 0xb6].fill(0xc0);
        }
        self.status = 0;
        self.busy_until = self.master_cycles;
        self.timer_control = 0x30;
        self.timer_a = 0;
        self.timer_b = 0;
        self.timer_a_remaining = 0;
        self.timer_b_remaining = 0;
    }

    /// Write one hardware port. Both data ports use the most recently selected
    /// bank/address, even when the address came from the other port pair.
    pub fn write(&mut self, port: usize, value: u8) {
        let time = self.clock().max(self.master_cycles);
        if port & 1 != 0 {
            self.sync_to(time);
        }
        self.write_at(port, value, time, 0);
    }

    /// Write at an explicit absolute access clock. GPGX rounds the BUSY end to
    /// the internal clock relative to the current frame's `origin`.
    pub fn write_at(&mut self, port: usize, value: u8, time: u64, origin: u64) {

        self.write_count += 1;
        if port < 4 {
            self.port_writes[port] += 1;
        }
        match port {
            0 | 2 => {
                if port == 0 {
                    self.port_a = value;
                } else {
                    self.port_b = value;
                }
                self.address = usize::from(value) | if port == 2 { 0x100 } else { 0 };
                self.lfsr = (self.lfsr << 1) ^ u16::from(self.lfsr & 0x8000 != 0);
            }
            1 | 3 => {
                self.busy_until = origin.wrapping_add(
                    (time.wrapping_sub(origin).div_ceil(YM2612_CLOCK_RATIO) + 32) * YM2612_CLOCK_RATIO,
                );
                let index = self.address;
                self.registers[index] = value;
                self.reg_writes[index] += 1;
                self.reg_last[index] = value;
                match index {
                    0x24 => self.timer_a = (u16::from(value) << 2) | (self.timer_a & 3),
                    0x25 => self.timer_a = (self.timer_a & 0x3fc) | u16::from(value & 3),
                    0x26 => self.timer_b = value,
                    0x27 => {
                        if value & 0x10 != 0 {
                            self.status &= !TIMER_A_OVERFLOW;
                        }
                        if value & 0x20 != 0 {
                            self.status &= !TIMER_B_OVERFLOW;
                        }
                        // A load rising edge reloads; rewriting an already-set
                        // load bit or changing the period leaves this count intact.
                        if value & 1 != 0 && self.timer_control & 1 == 0 {
                            self.timer_a_remaining = 1024 - u64::from(self.timer_a);
                        }
                        if value & 2 != 0 && self.timer_control & 2 == 0 {
                            self.timer_b_remaining = (256 - u64::from(self.timer_b)) * 16;
                        }
                        self.timer_control = value;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Reading does not acknowledge BUSY or timer flags. This models the
    /// discrete YM2612; integrated/enhanced variants have different port reads.
    pub fn read(&mut self, port: usize) -> u8 {
        let time = self.clock().max(self.master_cycles);
        self.sync_to(time);
        self.read_at(port, time)
    }

    /// Read at an explicit absolute access clock.
    pub fn read_at(&mut self, port: usize, time: u64) -> u8 {

        if port == 0 {
            self.status
                | if time < self.busy_until {
                    BUSY
                } else {
                    0
                }
        } else {
            0
        }
    }

    /// Diagnostic timer state: (timer A remaining samples, timer B remaining,
    /// samples elapsed, chip clock, timer A period register).
    pub fn timer_debug(&self) -> (u64, u64, u64, u64, u16) {
        (self.timer_a_remaining, self.timer_b_remaining, self.timer_samples, self.master_cycles, self.timer_a)
    }

    /// The system-driven clock (excludes any lead from access syncs).
    pub fn clock(&self) -> u64 {
        self.scheduled.unwrap_or(self.master_cycles)
    }

    /// Frame edge bookkeeping (reference sound_update): the next frame's
    /// sample counter restarts at the first grid point at or after the edge,
    /// so samples the chip already ran past it (a sound-CPU access overshooting
    /// the frame) are run again, ticking the timers twice.
    pub fn frame_edge(&mut self, edge: u64) {
        let grid = edge.div_ceil(SAMPLE_CLOCK_RATIO).max(1) * SAMPLE_CLOCK_RATIO;
        if self.master_cycles > grid {
            self.master_cycles = grid;
            self.timer_samples = grid / SAMPLE_CLOCK_RATIO;
        }
    }

    /// Run the chip up to `time` before an access (GPGX fm_update(cycles)).
    pub fn sync_to(&mut self, time: u64) {
        if time > self.master_cycles {
            self.advance(time - self.master_cycles);
        }
    }

    /// Advance by master clocks. GPGX fm_update rounds its sample horizon up;
    /// keep that absolute phase so splitting ticks cannot add extra timer counts.
    pub fn tick(&mut self, cycles: u32) {
        // Only the schedule moves: like the reference's fm_update, the chip
        // (timers) advances lazily when accessed, so a sound-CPU batch that
        // runs after the 68000's slice never observes future timer state.
        self.scheduled = Some(self.clock() + u64::from(cycles));
    }

    fn advance(&mut self, cycles: u64) {
        self.master_cycles += cycles;
        let samples = self.master_cycles.div_ceil(SAMPLE_CLOCK_RATIO);
        let elapsed = samples - self.timer_samples;
        self.timer_samples = samples;
        for (load, enable, period, remaining, flag) in [
            (
                1,
                4,
                1024 - u64::from(self.timer_a),
                &mut self.timer_a_remaining,
                TIMER_A_OVERFLOW,
            ),
            (
                2,
                8,
                (256 - u64::from(self.timer_b)) * 16,
                &mut self.timer_b_remaining,
                TIMER_B_OVERFLOW,
            ),
        ] {
            if self.timer_control & load != 0 && elapsed != 0 {
                if elapsed >= *remaining {
                    *remaining = period - (elapsed - *remaining) % period;
                    if self.timer_control & enable != 0 {
                        self.status |= flag;
                    }
                } else {
                    *remaining -= elapsed;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(fm: &mut Fm, address: u8, value: u8) {
        fm.write(0, address);
        fm.write(1, value);
    }

    #[test]
    fn shared_address_latch_keeps_banks_separate_and_accepts_cross_pair_data() {
        let mut fm = Fm::new();
        fm.write(0, 0x22);
        assert_eq!(
            fm.registers[0x22], 0,
            "address selection is not a data write"
        );
        fm.write(3, 0x08);
        assert_eq!(fm.registers[0x22], 0x08);
        fm.write(2, 0x22);
        fm.write(1, 0x99);
        assert_eq!(fm.registers[0x122], 0x99);
        assert_eq!(fm.registers[0x22], 0x08);
        fm.write(2, 0x24);
        fm.write(1, 0xff);
        assert_eq!(fm.timer_a, 0, "bank-1 register 0x24 is not timer A");
    }

    #[test]
    fn busy_is_data_only_rounded_to_internal_clock_and_not_read_to_clear() {
        let mut fm = Fm::new();
        fm.write(0, 0x22);
        assert_eq!(fm.read(0), 0);
        fm.tick(1);
        fm.write(1, 0);
        assert_eq!(fm.read(0), 0x80);
        assert_eq!(fm.read(0), 0x80);
        for port in 1..4 {
            assert_eq!(fm.read(port), 0);
        }
        fm.tick(1384); // ceil(1/42)*42 + 32*42 = 1386
        assert_eq!(fm.read(0), 0x80);
        fm.write(2, 0x30); // address write must not extend busy
        fm.tick(1);
        assert_eq!(fm.read(0), 0);
        fm.write(3, 0);
        fm.tick(1343);
        assert_eq!(fm.read(0), 0x80);
        fm.tick(1);
        assert_eq!(fm.read(0), 0);
    }

    #[test]
    fn timers_use_ten_bit_a_sixteen_sample_b_and_explicit_status_resets() {
        let mut fm = Fm::new();
        register(&mut fm, 0x24, 0xff);
        register(&mut fm, 0x25, 0xff);
        register(&mut fm, 0x26, 0xff);
        assert_eq!(fm.timer_a, 1023);
        register(&mut fm, 0x27, 0x0f);
        fm.tick(1008 * 15);
        assert_eq!(fm.read(0) & 3, 1);
        assert_eq!(fm.read(0) & 3, 1, "status read must not clear A");
        fm.tick(1); // GPGX rounds up to sample 16
        assert_eq!(fm.read(0) & 3, 3);
        register(&mut fm, 0x27, 0x1f);
        assert_eq!(fm.read(0) & 3, 2);
        register(&mut fm, 0x27, 0x2f);
        assert_eq!(fm.read(0) & 3, 0);
        fm.tick(1008);
        assert_eq!(
            fm.read(0) & 3,
            1,
            "reset bits are pulses, not overflow masks"
        );
    }

    #[test]
    fn load_edges_and_period_changes_preserve_the_in_flight_count() {
        let mut fm = Fm::new();
        register(&mut fm, 0x24, 0xff);
        register(&mut fm, 0x25, 0); // A period = four samples
        register(&mut fm, 0x27, 1); // count without enabling status
        fm.tick(1008 * 2);
        register(&mut fm, 0x25, 3); // new period = one; current count = two
        register(&mut fm, 0x27, 5); // enable without a new load edge
        fm.tick(1008);
        assert_eq!(fm.read(0) & 1, 0);
        fm.tick(1008);
        assert_eq!(fm.read(0) & 1, 1);
        register(&mut fm, 0x27, 0x15);
        fm.tick(1008);
        assert_eq!(fm.read(0) & 1, 1, "overflow reload uses the new period");
        register(&mut fm, 0x27, 0x10); // stop, reset flag
        fm.tick(1008 * 50);
        assert_eq!(fm.read(0) & 1, 0);
        register(&mut fm, 0x27, 1);
        fm.tick(1008 * 50);
        assert_eq!(fm.read(0) & 1, 0, "disabled status still allows counting");
    }

    #[test]
    fn split_ticks_do_not_change_timer_phase_and_reset_clears_busy_and_flags() {
        let mut whole = Fm::new();
        register(&mut whole, 0x24, 0xff);
        register(&mut whole, 0x25, 3);
        register(&mut whole, 0x27, 5);
        let mut split = whole.clone();
        whole.tick(1008);
        for _ in 0..1008 {
            split.tick(1);
        }
        assert_eq!(whole.read(0), split.read(0));
        assert_eq!(whole.timer_a_remaining, split.timer_a_remaining);
        split.reset();
        assert_eq!(split.read(0), 0);
        split.tick(1008 * 100);
        assert_eq!(split.read(0), 0);
    }
}
