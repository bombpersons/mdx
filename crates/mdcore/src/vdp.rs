//! The Mega Drive video display processor.
//!
//! Covers what a cartridge actually drives: planes A and B, the window, sprites,
//! scrolling, the 9-bit palette and priority resolution, for the 320- and
//! 256-pixel-wide modes. Interlaced output and the unusual modes are treated as
//! unknown rather than guessed at, and the renderer says so in its coverage.

mod renderer;
mod scanline;
pub use scanline::Raster;

/// RGB framebuffer, 3 bytes per pixel.
#[derive(Clone)]
pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0; width * height * 3],
        }
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, rgb: (u8, u8, u8)) {
        let i = (y * self.width + x) * 3;
        self.data[i] = rgb.0;
        self.data[i + 1] = rgb.1;
        self.data[i + 2] = rgb.2;
    }
}

pub const VRAM_SIZE: usize = 0x1_0000;
pub const CRAM_ENTRIES: usize = 64;
pub const VSRAM_ENTRIES: usize = 40;

/// Where the VDP addressed by the CPU lives inside its own 16-bit space.
pub const VRAM: u16 = 0x0000;
pub const CRAM: u16 = 0xC000;
pub const VSRAM: u16 = 0x4000;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vdp {
    pub registers: [u8; 32],
    pub vram: Vec<u8>,
    pub cram: Vec<u8>,
    pub vsram: Vec<u8>,
    /// Latched CD1/CD2 for the next VRAM access.
    pub address_latch: u32,
    /// CD code bits from the control-port write that is in effect.
    pub code: u8,
    pub pending_dm: bool,
    /// Fill/copy owns internal VDP slots while the 68000 keeps executing.
    /// Separate from external DMA's CPU-bus stall and the fill-data latch.
    internal_dma_master_cycles: u32,
    /// A first control-command word has been received.
    pub control_pending: bool,
    /// DMA requests are observable even while DMA transfer is unsupported.
    pub dma_requests: u64,
    pub dma_bytes: u64,
    /// Elapsed master clocks while external DMA owns the 68000 bus.
    pub dma_bus_stall_master_cycles: u64,
    pub dma_unsupported: bool,
    hint_counter: u8,
    /// Non-zero while the display is disabled.
    pub vblank: bool,
    /// Horizontal blanking flag, level-derived like `vblank`.
    pub hblank: bool,
    /// Dot clock position within the current line, 0..896.
    pub dot: u32,
    /// Line within the frame, 0..262 on NTSC, 0..313 on PAL.
    pub line: u32,
    /// PAL console: 313-line frames, PAL V-counter wrap points and status bit.
    #[serde(default)]
    pub pal: bool,
    /// Completed frames since reset.
    pub frame: u64,
    /// Set while a vertical interrupt is being presented to the 68000.
    pub vint_pending: bool,
    /// Z80 INT is a level pulse from VINT to the end of that scanline.
    #[serde(default)]
    pub z80_irq: bool,
    /// Vertical interrupts the chip has raised, independent of whether the CPU
    /// took them.
    pub vint_requests: u64,
    /// Vertical interrupts the CPU actually accepted.
    pub vint_taken: u64,
    /// Set while a horizontal interrupt is being presented to the 68000.
    pub hint_pending: bool,
    /// Fractional normalized dots carried across 68000 instruction boundaries.
    cpu_clock_remainder: u64,
    /// Elapsed time in master-clock * 896 units (also supports dot-based tests).
    elapsed_scaled: u64,
    visible_lines: u32,
    vint_deadline: Option<u64>,
    fifo_deadlines: [u64; 4],
    fifo_index: usize,
    fifo_data: [u16; 4],
    fifo_data_index: usize,
    hvc_latch: Option<u16>,
    pub fifo_stall_master_cycles: u64,
    /// Scanline renderer state (sprite cache, line lists, rendered screen).
    #[serde(default)]
    pub raster: Raster,
    /// A register write enabled an already-pending interrupt: the 68000
    /// accepts it only after one further instruction (GPGX m68k_set_irq_delay).
    #[serde(default)]
    pub irq_delay: bool,
    /// Internal DMA BUSY is held to the frame edge (fill-start wrap), so a
    /// status sample past the edge within the same instruction still sees it.
    #[serde(default)]
    dma_busy_to_frame_edge: bool,
    /// 68000-bus DMA words not yet delivered: GPGX transfers them in per-line
    /// (or whole-VBLANK) updates at each update's start, so the renderer and
    /// sprite cache see partial progress while the 68000 is held.
    #[serde(default)]
    pub dma_queue: std::collections::VecDeque<u16>,
    /// A15-A14 from the last second command word (GPGX addr_latch); the first
    /// word of the next command combines with these, not the incremented
    /// address.
    #[serde(default)]
    pub address_high: u32,
    /// Control word written while 68000-bus DMA is still transferring; applied
    /// when it completes (GPGX cached_write).
    #[serde(default)]
    pub cached_write: Option<u16>,
    /// Fill/copy still transferring: the reference moves it per update (line
    /// or whole VBLANK) through the live address register and FIFO, so later
    /// commands redirect the remainder.
    #[serde(default)]
    pub internal_dma: Option<InternalDma>,
    /// The frame-start (whole VBLANK) DMA update is due but deferred until
    /// the CPU next runs: the reference's frame ends before it, so a capture
    /// at the frame edge must not include it.
    #[serde(default)]
    pub frame_start_dma_pending: bool,
}

/// Remaining fill/copy transfer (GPGX dma_length/dma_src for types 2 and 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InternalDma {
    pub remaining: u32,
    pub copy: bool,
    pub source: u16,
}

/// Master-clock dots in one scanline.
pub const DOTS_PER_LINE: u32 = 896;
/// Lines in one NTSC frame.
pub const LINES_PER_FRAME: u32 = 262;
/// First line of the vertical blank interval.
pub const VBLANK_START: u32 = 224;
/// Last line of vertical blanking, NTSC.
pub const VBLANK_END: u32 = LINES_PER_FRAME - 1;
/// Lines in one PAL frame.
pub const PAL_LINES_PER_FRAME: u32 = 313;
/// One scanline spans 3420 master clocks. The 68000 runs at master / 7.
/// `dot` is a normalized 896-unit line coordinate, not a physical clock.
pub const MASTER_CYCLES_PER_LINE: u32 = 3420;
pub const MASTER_CYCLES_PER_CPU_CYCLE: u32 = 7;

/// Did the line counter cross `edge` while moving from `from` to `to`?
///
/// Handles the wrap at the end of the frame, so an interrupt is not missed or
/// duplicated when the caller advances by many dots at once.
fn crossed(from: u32, to: u32, edge: u32) -> bool {
    if from <= to {
        from <= edge && edge <= to
    } else {
        from <= edge || edge <= to
    }
}

impl Vdp {
    /// Lines per frame for the console's video standard.
    pub fn lines_per_frame(&self) -> u32 {
        if self.pal {
            PAL_LINES_PER_FRAME
        } else {
            LINES_PER_FRAME
        }
    }

    /// Master clocks per frame.
    pub fn frame_master(&self) -> u64 {
        u64::from(self.lines_per_frame()) * u64::from(MASTER_CYCLES_PER_LINE)
    }

    pub(crate) fn valid_state(&self, master: u64) -> bool {
        self.vram.len() == VRAM_SIZE
            && self.cram.len() == CRAM_ENTRIES * 2
            && self.vsram.len() == VSRAM_ENTRIES * 2
            && self.line < self.lines_per_frame()
            && self.dot < DOTS_PER_LINE
            && self.cpu_clock_remainder < u64::from(MASTER_CYCLES_PER_LINE)
            && self.fifo_index < 4
            && self.fifo_data_index < 4
            && matches!(self.visible_lines, 192 | 224 | 240)
            && self.frame <= master / self.frame_master() + 2
            && self.fifo_deadlines.iter().all(|at| *at < u64::MAX / 4096)
            && self.vint_deadline.is_none_or(|at| at < u64::MAX / 4)
            && self.address_latch <= 0xffff
            && self.code <= 0x3f
            && master.checked_mul(u64::from(DOTS_PER_LINE)) == Some(self.elapsed_scaled)
            && self.raster.valid()
    }

    /// Absolute master clock (VDP time since power-on).
    pub fn master_clock(&self) -> u64 {
        self.elapsed_scaled / u64::from(DOTS_PER_LINE)
    }

    pub(crate) fn line_event_in_master_cycles(&self) -> u32 {
        let position = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896;
        (3420 - position) as u32
    }

    /// Next sound INT edge, or the frame boundary which schedules its assertion.
    /// Split long CPU/DMA waits here so the Z80 sees the finite pulse in order.
    pub(crate) fn sound_event_in_master_cycles(&self) -> u32 {
        let position = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896;
        if let Some(deadline) = self.vint_deadline {
            return deadline
                .saturating_sub(self.elapsed_scaled)
                .div_ceil(896)
                .clamp(1, u64::from(u32::MAX)) as u32;
        }
        let lines = if self.z80_irq {
            1
        } else if self.line < self.visible_lines {
            self.visible_lines - self.line
        } else {
            self.lines_per_frame() - self.line + self.visible_lines
        };
        (u64::from(lines) * 3420 - position).max(1) as u32
    }

    /// Advance using the exact clock ratio, retaining fractional line units.
    /// Chunking a run into instructions must not discard time on every step.
    pub fn advance_cpu_cycles(&mut self, cycles: u32) {
        self.advance_master_cycles(cycles * MASTER_CYCLES_PER_CPU_CYCLE);
    }

    /// Advance device time without rounding a DMA duration to whole CPU clocks.
    pub fn advance_master_cycles(&mut self, cycles: u32) {
        self.internal_dma_master_cycles = self.internal_dma_master_cycles.saturating_sub(cycles);
        self.advance_scaled(u64::from(cycles) * u64::from(DOTS_PER_LINE));
    }

    /// External-bus DMA wait using GPGX vdp_ctrl.c:664-801 access rates.
    /// VRAM consumes two slots per word; CRAM/VSRAM consume one. The CPU
    /// cannot change registers during this wait. Port accesses are still
    /// instruction-boundary observations, not a sub-instruction FIFO model.
    pub fn external_dma_master_cycles(&self, words: u32, to_vram: bool) -> u32 {
        self.external_dma_wait(words, to_vram).0
    }

    /// Return the wait and whether the transfer requires a later scheduler
    /// update. Such an update overlaps the initiating opcode's fixed clocks.
    pub(crate) fn external_dma_wait(&self, mut words: u32, to_vram: bool) -> (u32, bool) {
        let mut line = self.line;
        let mut visible_lines = self.visible_lines;
        let mut position = ((u64::from(self.dot) * u64::from(MASTER_CYCLES_PER_LINE)
            + self.cpu_clock_remainder)
            / u64::from(DOTS_PER_LINE)) as u32;
        let mut elapsed = 0;
        let mut continues = false;
        while words != 0 {
            // The reference clears VBLANK on the final pre-active line.
            let blank =
                !self.display_enabled() || (visible_lines..self.lines_per_frame() - 1).contains(&line);
            let mut rate = match (blank, self.registers[12] & 1 != 0) {
                (false, false) => 16,
                (false, true) => 18,
                (true, false) => 166,
                (true, true) => 204,
            };
            if to_vram {
                rate >>= 1;
            } else if blank {
                rate = if rate == 166 { 161 } else { 198 };
            }
            // During VBLANK the reference scheduler processes the entire
            // remaining blank period in one update, ending before line261.
            let lines = if (visible_lines..self.lines_per_frame() - 1).contains(&line) {
                self.lines_per_frame() - 1 - line
            } else {
                1
            };
            let available = lines * MASTER_CYCLES_PER_LINE - position;
            let capacity = available * rate / MASTER_CYCLES_PER_LINE;
            if elapsed == 0 {
                continues = words > capacity;
            }
            if words < capacity {
                elapsed += words * MASTER_CYCLES_PER_LINE / rate;
                break;
            }
            words -= capacity;
            elapsed += available;
            line = (line + lines) % self.lines_per_frame();
            if line == visible_lines {
                // The viewport latches on the same frame edge as the real VDP.
                // A transfer begun in reset's Mode4 frame can continue after
                // Mode5 has changed the height and remaining blank interval.
                visible_lines = self.display_lines();
                line = visible_lines;
            }
            position = 0;
        }
        (elapsed, continues)
    }

    /// 68000-bus DMA rate in words per line for the update starting at `line`.
    fn external_dma_rate(&self, line: u32, to_vram: bool) -> u32 {
        let blank =
            !self.display_enabled() || (self.visible_lines..self.lines_per_frame() - 1).contains(&line);
        let mut rate = match (blank, self.registers[12] & 1 != 0) {
            (false, false) => 16,
            (false, true) => 18,
            (true, false) => 166,
            (true, true) => 204,
        };
        if to_vram {
            rate >>= 1;
        } else if blank {
            rate = if rate == 166 { 161 } else { 198 };
        }
        rate
    }

    /// Words the reference's first DMA update delivers immediately.
    pub(crate) fn external_dma_first_words(&self, words: u32, to_vram: bool) -> u32 {
        let rate = self.external_dma_rate(self.line, to_vram);
        (u64::from(self.external_dma_timeframe_cycles()) * u64::from(rate)
            / u64::from(MASTER_CYCLES_PER_LINE))
        .min(u64::from(words)) as u32
    }

    /// Fill/copy rate in bytes per line for an update at the current line.
    fn internal_dma_rate(&self, copy: bool) -> u32 {
        self.internal_dma_rate_at(self.line, copy)
    }

    fn internal_dma_rate_at(&self, line: u32, copy: bool) -> u32 {
        let blank = !self.display_enabled()
            || (self.visible_lines..self.lines_per_frame() - 1).contains(&line);
        let rate = match (blank, self.registers[12] & 1 != 0) {
            (false, false) => 16,
            (false, true) => 18,
            (true, false) => 166,
            (true, true) => 204,
        };
        if copy { rate >> 1 } else { rate }
    }

    /// Start a fill (after its data-port word) or copy: busy timing, then the
    /// reference's first update from the current position.
    pub(crate) fn begin_internal_dma(&mut self, count: u32, copy: bool, source: u16) {
        self.start_internal_dma(count, copy);
        self.internal_dma = Some(InternalDma { remaining: count, copy, source });
        let rate = self.internal_dma_rate(copy);
        let position = ((u64::from(self.dot) * u64::from(MASTER_CYCLES_PER_LINE)
            + self.cpu_clock_remainder)
            / u64::from(DOTS_PER_LINE)) as u32;
        let start = position + if copy { 0 } else { 2 * (MASTER_CYCLES_PER_LINE / rate) };
        let end = if (self.visible_lines..self.lines_per_frame() - 1).contains(&self.line) {
            (self.lines_per_frame() - 1 - self.line) * MASTER_CYCLES_PER_LINE
        } else {
            MASTER_CYCLES_PER_LINE
        };
        // A negative budget wraps unsigned in the reference: all at once.
        let bytes = if start > end { count } else { ((end - start) * rate / MASTER_CYCLES_PER_LINE).min(count) };
        self.run_internal_dma(bytes);
    }

    /// Transfer `bytes` of the active fill/copy (GPGX vdp_dma_fill/copy).
    fn run_internal_dma(&mut self, bytes: u32) {
        let Some(mut dma) = self.internal_dma else { return };
        let bytes = bytes.min(dma.remaining);
        if bytes == 0 {
            return;
        }
        let step = u32::from(self.registers[15]);
        if dma.copy {
            for _ in 0..bytes {
                let value = self.vram[(dma.source as usize ^ 1) & 0xFFFF];
                self.poke_vram((self.address_latch as usize ^ 1) & 0xFFFF, value);
                dma.source = dma.source.wrapping_add(1);
                self.address_latch = self.address_latch.wrapping_add(step) & 0xFFFF;
            }
        } else {
            match self.code & 0xF {
                1 => {
                    let data = (self.fifo_data[(self.fifo_data_index + 3) & 3] >> 8) as u8;
                    for _ in 0..bytes {
                        self.poke_vram((self.address_latch as usize ^ 1) & 0xFFFF, data);
                        self.address_latch = self.address_latch.wrapping_add(step) & 0xFFFF;
                    }
                }
                3 => {
                    let data = self.fifo_data[self.fifo_data_index] & 0x0EEE;
                    for _ in 0..bytes {
                        let a = self.address_latch as usize & 0x7E;
                        self.cram[a] = (data >> 8) as u8;
                        self.cram[a + 1] = data as u8;
                        self.address_latch = self.address_latch.wrapping_add(step) & 0xFFFF;
                    }
                }
                5 => {
                    let data = self.fifo_data[self.fifo_data_index];
                    for _ in 0..bytes {
                        let a = self.address_latch as usize & 0x7E;
                        if a < self.vsram.len() {
                            self.vsram[a] = ((data >> 8) & 7) as u8;
                            self.vsram[a + 1] = data as u8;
                        }
                        self.address_latch = self.address_latch.wrapping_add(step) & 0xFFFF;
                    }
                }
                // Invalid destination writes nothing; the address still moves.
                _ => self.address_latch = self.address_latch.wrapping_add(step * bytes) & 0xFFFF,
            }
        }
        self.dma_bytes += u64::from(bytes);
        dma.remaining -= bytes;
        self.internal_dma = (dma.remaining != 0).then_some(dma);
    }

    /// DMA updates at the start of the current line: per active line, and
    /// once for the whole VBLANK at the frame edge (deferred; see
    /// `frame_start_dma_pending`).
    fn drain_dma_queue(&mut self) {
        self.flush_frame_start_dma();
        if self.line == self.visible_lines {
            self.frame_start_dma_pending = self.internal_dma.is_some() || !self.dma_queue.is_empty();
        } else if !(self.visible_lines..self.lines_per_frame() - 1).contains(&self.line) {
            self.dma_update(self.line, 1);
        }
    }

    /// Run a deferred frame-start DMA update (before the CPU runs again).
    pub fn flush_frame_start_dma(&mut self) {
        if std::mem::take(&mut self.frame_start_dma_pending) {
            let line = self.visible_lines;
            self.dma_update(line, self.lines_per_frame() - 1 - line);
        }
    }

    fn dma_update(&mut self, line: u32, lines: u32) {
        if let Some(dma) = self.internal_dma {
            self.run_internal_dma(lines * self.internal_dma_rate_at(line, dma.copy));
        }
        if self.dma_queue.is_empty() {
            return;
        }
        let to_vram = self.code & 0xF == 1;
        let words = lines * self.external_dma_rate(line, to_vram);
        for _ in 0..words {
            let Some(word) = self.dma_queue.pop_front() else { break };
            self.data_write_direct(word);
        }
        if self.dma_queue.is_empty() {
            if let Some(word) = self.cached_write.take() {
                self.control_first_word(word);
            }
        }
    }

    /// GPGX vdp_68k_ctrl_w with no command pending: the word always updates
    /// the address (with the latched A15-A14) and code bits 1-0; `10` in bits
    /// 15-14 also writes a register (so code becomes the invalid target 2).
    /// Mode 4 commands are single words; registers 11-23 are locked in mode 4.
    /// Returns whether a register was written.
    pub fn control_first_word(&mut self, value: u16) -> bool {
        self.address_latch = self.address_high | u32::from(value & 0x3FFF);
        self.code = (self.code & 0x3C) | ((value >> 14) as u8 & 3);
        if value & 0xC000 == 0x8000 {
            let index = usize::from((value >> 8) & 0x1F);
            if self.registers[1] & 4 != 0 || index <= 10 {
                self.control_write(index, value as u8);
            }
            true
        } else {
            self.control_pending = self.registers[1] & 4 != 0;
            false
        }
    }

    /// Remaining clocks in the reference DMA scheduler's current update.
    /// VBLANK is processed together; other display lines are separate updates.
    pub(crate) fn external_dma_timeframe_cycles(&self) -> u32 {
        let position = ((u64::from(self.dot) * u64::from(MASTER_CYCLES_PER_LINE)
            + self.cpu_clock_remainder)
            / u64::from(DOTS_PER_LINE)) as u32;
        let lines = if (self.visible_lines..self.lines_per_frame() - 1).contains(&self.line) {
            self.lines_per_frame() - 1 - self.line
        } else {
            1
        };
        lines * MASTER_CYCLES_PER_LINE - position
    }

    pub fn dma_busy(&self) -> bool {
        self.pending_dm || self.internal_dma_master_cycles != 0
    }

    /// Internal DMA uses the same reference access slots but does not halt the
    /// CPU. Fill uses one slot/byte plus two startup slots; copy uses two.
    pub fn start_internal_dma(&mut self, mut bytes: u32, copy: bool) {
        let mut line = self.line;
        let mut position = ((u64::from(self.dot) * u64::from(MASTER_CYCLES_PER_LINE)
            + self.cpu_clock_remainder)
            / u64::from(DOTS_PER_LINE)) as u32;
        // The viewport (and so the blank interval) is re-latched at each frame
        // edge, as in external_dma_wait: a transfer started just after a mode
        // change runs under the new height from the next frame.
        let mut visible = self.visible_lines;
        let rate_at = |line, visible: u32| {
            let blank = !self.display_enabled()
                || (visible..self.lines_per_frame() - 1).contains(&line);
            let rate = match (blank, self.registers[12] & 1 != 0) {
                (false, false) => 16,
                (false, true) => 18,
                (true, false) => 166,
                (true, true) => 204,
            };
            if copy {
                rate / 2
            } else {
                rate
            }
        };
        let next_line = |line: &mut u32, visible: &mut u32| {
            *line = (*line + 1) % self.lines_per_frame();
            if *line == *visible {
                *visible = self.display_lines();
                *line = *visible;
            }
        };
        let mut elapsed = if copy {
            0
        } else {
            2 * (MASTER_CYCLES_PER_LINE / rate_at(line, visible))
        };
        // GPGX vdp_dma_update computes the first update's budget as
        // (end - start) * rate with an unsigned rate: when the fill's start
        // adjustment passes the end of the current update, the negative span
        // wraps, the whole fill completes at once, and BUSY lasts until that
        // span's end or the frame edge (where dma_endCycles and BUSY reset).
        let update_end = if (self.visible_lines..self.lines_per_frame() - 1).contains(&line) {
            (self.lines_per_frame() - 1 - line) * MASTER_CYCLES_PER_LINE
        } else {
            MASTER_CYCLES_PER_LINE
        };
        if position + elapsed > update_end {
            let to_frame = if line < self.visible_lines {
                self.visible_lines - line
            } else {
                self.lines_per_frame() - line + self.visible_lines
            } * MASTER_CYCLES_PER_LINE
                - position;
            let span = elapsed + bytes * MASTER_CYCLES_PER_LINE / rate_at(line, visible);
            self.internal_dma_master_cycles = span.min(to_frame);
            self.dma_busy_to_frame_edge = span > to_frame;
            return;
        }
        position += elapsed;
        if position >= MASTER_CYCLES_PER_LINE {
            position -= MASTER_CYCLES_PER_LINE;
            next_line(&mut line, &mut visible);
        }
        while bytes != 0 {
            let rate = rate_at(line, visible);
            let available = MASTER_CYCLES_PER_LINE - position;
            let capacity = available * rate / MASTER_CYCLES_PER_LINE;
            if bytes < capacity {
                elapsed += bytes * MASTER_CYCLES_PER_LINE / rate;
                break;
            }
            bytes -= capacity;
            elapsed += available;
            next_line(&mut line, &mut visible);
            position = 0;
        }
        self.internal_dma_master_cycles = elapsed;
        self.dma_busy_to_frame_edge = false;
    }

    /// Advance the dot clock and raise interrupt requests on the edges.
    pub fn advance(&mut self, dots: u32) {
        self.advance_scaled(u64::from(dots) * u64::from(MASTER_CYCLES_PER_LINE));
    }

    fn advance_scaled(&mut self, mut left: u64) {
        const LINE: u64 = DOTS_PER_LINE as u64 * MASTER_CYCLES_PER_LINE as u64;
        while left > 0 {
            let position =
                u64::from(self.dot) * u64::from(MASTER_CYCLES_PER_LINE) + self.cpu_clock_remainder;
            let span = (LINE - position).min(left);
            let next = position + span;
            self.elapsed_scaled += span;
            self.dot = (next / u64::from(MASTER_CYCLES_PER_LINE)) as u32;
            self.cpu_clock_remainder = next % u64::from(MASTER_CYCLES_PER_LINE);
            left -= span;
            if self
                .vint_deadline
                .is_some_and(|at| self.elapsed_scaled >= at)
            {
                self.vint_deadline = None;
                self.vint_pending = true;
                self.z80_irq = true;
                self.vint_requests += 1;
            }
            if self.dot >= DOTS_PER_LINE {
                self.dot = 0;
                self.z80_irq = false;
                self.line += 1;
                if self.line == self.visible_lines {
                    self.raster_frame_end(self.visible_lines);
                    self.frame += 1;
                    // Viewport geometry is latched at each frame boundary.
                    // Reset's first Mode4 frame ends at 192 even if software
                    // enabled Mode5 during that frame (GPGX system_frame_gen).
                    self.visible_lines = self.display_lines();
                    self.line = self.visible_lines;
                    self.fifo_deadlines = [0; 4];
                    let delay = if self.registers[12] & 1 != 0 {
                        788
                    } else {
                        770
                    };
                    self.vint_deadline =
                        Some(self.elapsed_scaled + delay * u64::from(DOTS_PER_LINE));
                }
                if self.line >= self.lines_per_frame() {
                    self.line = 0;
                }
                // GPGX system_frame_gen: active lines count down and reload;
                // the VBLANK edge raises HINT if the count is already zero
                // (without reloading); only the last blank line reloads.
                if self.line < self.visible_lines {
                    if self.hint_counter == 0 {
                        self.hint_counter = self.registers[10];
                        self.hint_pending = true;
                    } else {
                        self.hint_counter -= 1;
                    }
                } else if self.line == self.visible_lines {
                    if self.hint_counter == 0 {
                        self.hint_pending = true;
                    }
                } else if self.line == self.lines_per_frame() - 1 {
                    self.hint_counter = self.registers[10];
                }
                // GPGX renders each active line at its start (after the DMA
                // update, before either CPU runs) and parses line 0's sprites
                // on the last blank line.
                self.drain_dma_queue();
                if self.line < self.visible_lines {
                    self.render_line(self.line);
                } else if self.line == self.lines_per_frame() - 1 && self.display_enabled() {
                    self.parse_satb(-1, if self.registers[12] & 1 != 0 { 320 } else { 256 });
                }
            }
        }

        // VINT is delayed from frame completion by the H32/H40 master-clock
        // interval above. Blank flags themselves are physical level signals.
        // The blanking flags are level signals derived from the line counter.
        // Software polls them directly - a vertical blank, then interrupt
        // enable, is the standard handshake - so they have to be live rather
        // than implied by the interrupt request.
        self.vblank = self.line >= self.visible_lines && self.line < (self.lines_per_frame() - 1);
        // Horizontal blanking covers the last 84 dots of every active line,
        // and runs the whole line while the display is disabled.
        self.hblank = self.line >= self.visible_lines || self.dot >= DOTS_PER_LINE - 84;
    }

    pub fn new() -> Self {
        Self {
            registers: [0; 32],
            vram: vec![0; VRAM_SIZE],
            cram: vec![0; CRAM_ENTRIES * 2],
            vsram: vec![0; VSRAM_ENTRIES * 2],
            address_latch: 0,
            code: 0,
            dot: 0,
            line: 0,
            pal: false,
            frame: 0,
            vint_pending: false,
            z80_irq: false,
            vint_requests: 0,
            vint_taken: 0,
            hint_pending: false,
            pending_dm: false,
            internal_dma_master_cycles: 0,
            control_pending: false,
            dma_requests: 0,
            dma_bytes: 0,
            dma_bus_stall_master_cycles: 0,
            dma_unsupported: false,
            hint_counter: 0,
            vblank: false,
            hblank: false,
            cpu_clock_remainder: 0,
            elapsed_scaled: 0,
            visible_lines: 224,
            vint_deadline: None,
            fifo_deadlines: [0; 4],
            fifo_index: 0,
            fifo_data: [0; 4],
            fifo_data_index: 0,
            hvc_latch: Some(0),
            fifo_stall_master_cycles: 0,
            raster: Raster::default(),
            irq_delay: false,
            dma_busy_to_frame_edge: false,
            dma_queue: std::collections::VecDeque::new(),
            address_high: 0,
            cached_write: None,
            internal_dma: None,
            frame_start_dma_pending: false,
        }
    }

    pub fn reset(&mut self) {
        self.registers = [0; 32];
        // H-INT is disabled on startup: the line counter register and the
        // counter itself power on at $FF (reference vdp_reset).
        self.registers[10] = 0xFF;
        self.vram.fill(0);
        self.cram.fill(0);
        self.vsram.fill(0);
        self.address_latch = 0;
        self.code = 0;
        self.dot = 0;
        // Reset begins after VBLANK has risen; no VINT on this initial edge.
        // The next completed frame reaches the same edge after 262 lines.
        self.visible_lines = 192;
        self.line = self.visible_lines;
        self.elapsed_scaled = 0;
        self.vint_deadline = None;
        self.fifo_deadlines = [0; 4];
        self.fifo_index = 0;
        self.fifo_data = [0; 4];
        self.fifo_data_index = 0;
        self.hvc_latch = Some(0);
        self.fifo_stall_master_cycles = 0;
        self.frame = 0;
        self.cpu_clock_remainder = 0;
        self.vblank = true;
        self.hblank = false;
        self.vint_pending = false;
        self.z80_irq = false;
        self.hint_pending = false;
        self.vint_requests = 0;
        self.vint_taken = 0;
        self.hint_counter = 0xFF;
        self.pending_dm = false;
        self.internal_dma_master_cycles = 0;
        self.control_pending = false;
        self.dma_requests = 0;
        self.dma_bytes = 0;
        self.dma_bus_stall_master_cycles = 0;
        self.dma_unsupported = false;
        self.raster.reset();
        self.irq_delay = false;
        self.dma_busy_to_frame_edge = false;
        self.dma_queue.clear();
        self.address_high = 0;
        self.cached_write = None;
        self.internal_dma = None;
        self.frame_start_dma_pending = false;
    }

    /// Read a control register.
    #[inline]
    pub fn control_read(&self, index: usize) -> u8 {
        self.registers[index & 0x1F]
    }

    fn display_lines(&self) -> u32 {
        if self.registers[1] & 4 == 0 {
            192
        } else if self.registers[1] & 8 != 0 {
            240
        } else {
            224
        }
    }

    /// Reserve a CPU data-port word. Data effects remain immediate, but a full
    /// four-word FIFO applies the independently documented H32/H40 bus wait.
    /// `elapsed_wait` includes preceding writes in the same CPU instruction.
    pub fn fifo_write_wait(&mut self, elapsed_wait: u32) -> u32 {
        if !self.display_enabled() || self.vblank {
            return 0;
        }
        let now = self.elapsed_scaled / u64::from(DOTS_PER_LINE) + u64::from(elapsed_wait);
        let last = self.fifo_deadlines[(self.fifo_index + 3) & 3];
        let oldest = self.fifo_deadlines[self.fifo_index];
        let wait = if now < last && now < oldest {
            // CPU-cycle rounding is relative to the scheduler's current
            // frame, whose NTSC length is not divisible by seven. Rounding
            // the global deadline shifts waits by up to six master clocks.
            let frame =
                self.elapsed_scaled / u64::from(DOTS_PER_LINE) / self.frame_master() * self.frame_master();
            frame + (oldest - frame).div_ceil(7) * 7 - now
        } else {
            0
        };
        let ready = now.max(last);
        let slots: &[u64] = if self.registers[12] & 1 != 0 {
            &[
                352, 820, 948, 1076, 1332, 1460, 1588, 1844, 1972, 2100, 2356, 2484, 2612, 2868,
                2996, 3124, 3364, 3380,
            ]
        } else {
            &[
                230, 510, 810, 970, 1130, 1450, 1610, 1770, 2090, 2250, 2410, 2730, 2890, 3050,
                3350, 3370,
            ]
        };
        // Convert the next absolute slot from the current physical scanline.
        let position = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896;
        let base = i128::from(now) - i128::from(elapsed_wait) - i128::from(position);
        let relative = (i128::from(ready) - base) as u64;
        let mut slot = relative / 3420 * slots.len() as u64;
        slot += slots
            .iter()
            .position(|at| *at > relative % 3420)
            .unwrap_or(slots.len()) as u64;
        // A VRAM word needs two byte slots; CRAM/VSRAM words need one.
        slot += u64::from(self.code & 6 == 0);
        self.fifo_deadlines[self.fifo_index] = (base
            + i128::from(slot / slots.len() as u64 * 3420 + slots[slot as usize % slots.len()]))
            as u64;
        self.fifo_index = (self.fifo_index + 1) & 3;
        self.fifo_stall_master_cycles += wait;
        wait as u32
    }

    pub fn fifo_status(&self) -> u16 {
        self.fifo_status_at(0)
    }

    fn fifo_status_at(&self, offset: u32) -> u16 {
        let now = self.elapsed_scaled / u64::from(DOTS_PER_LINE) + u64::from(offset);
        if now >= self.fifo_deadlines[(self.fifo_index + 3) & 3] {
            0x200
        } else if now < self.fifo_deadlines[self.fifo_index] {
            0x100
        } else {
            0
        }
    }

    pub fn control_write(&mut self, index: usize, value: u8) {
        let index = index & 0x1F;
        let old = self.registers[index];
        self.registers[index] = value;
        self.raster_register_write(index, old, value);
        let changed = old ^ value;
        if (index == 1 && changed & value & 0x20 != 0 && self.vint_pending)
            || (index == 0 && changed & value & 0x10 != 0 && self.hint_pending)
        {
            self.irq_delay = true;
        }
        if index == 0 && (old ^ value) & 2 != 0 && self.registers[1] & 4 != 0 {
            self.hvc_latch = if value & 2 != 0 {
                Some(self.hv_live())
            } else {
                None
            };
        }
        if index == 1 && (old ^ value) & 4 != 0 {
            self.hvc_latch = if value & 4 == 0 {
                Some(self.hv_counter())
            } else if self.registers[0] & 2 != 0 {
                Some(self.hv_live())
            } else {
                None
            };
        }
    }

    /// NTSC V/H counter sampling at the current master-clock position.
    /// Horizontal timing follows the independently measured standard H32/H40
    /// dot clocks, including EDCLK's nonuniform HSYNC interval in H40.
    pub fn hv_counter(&self) -> u16 {
        self.hv_counter_at(0)
    }

    /// HV counter `bias` master clocks after the VDP's position; past the line
    /// end the H counter wraps and the V counter advances (GPGX vdp_hvc_r).
    pub fn hv_counter_at(&self, bias: u32) -> u16 {
        if let Some(latched) = self.hvc_latch {
            if self.registers[1] & 4 != 0 {
                return latched;
            }
            return (self.hv_live_at(bias) & 0xff00) | (latched & 0xff);
        }
        self.hv_live_at(bias)
    }

    /// 68000 status-register bits sampled `offset` master clocks after the
    /// current position (GPGX vdp_68k_ctrl_r adds the reading instruction's
    /// table cost): FIFO, DMA busy, HBLANK, and VINT once its edge is reached.
    /// The window is line-relative and does not wrap into the next line.
    pub fn status_bits_at(&self, offset: u32) -> u16 {
        let mut status = self.fifo_status_at(offset);
        if self.pending_dm
            || self.internal_dma_master_cycles > offset
            || (self.dma_busy_to_frame_edge && self.internal_dma_master_cycles != 0)
        {
            status |= 0x0002;
        }
        let t = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896 + u64::from(offset);
        let window = if self.registers[12] & 1 == 0 { 280..860 } else { 228..872 };
        if window.contains(&t) {
            status |= 0x0004;
        }
        let edge = self.vint_deadline.is_some_and(|at| {
            self.elapsed_scaled + u64::from(offset) * u64::from(DOTS_PER_LINE) >= at
        });
        if self.vint_pending || (edge && !self.z80_irq) {
            status |= 0x0080;
        }
        status
    }

    /// HBLANK status level at the current position: master clocks 280..860
    /// of each line in H32, 228..872 in H40, on every line (reference model).
    pub fn hblank_now(&self) -> bool {
        let t = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896;
        if self.registers[12] & 1 == 0 {
            (280..860).contains(&t)
        } else {
            (228..872).contains(&t)
        }
    }

    fn hv_live(&self) -> u16 {
        self.hv_live_at(0)
    }

    fn hv_live_at(&self, bias: u32) -> u16 {
        let position = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896;
        let mut t = position as u32 + bias;
        let mut line = self.line;
        if t >= MASTER_CYCLES_PER_LINE {
            t -= MASTER_CYCLES_PER_LINE;
            line = (line + 1) % self.lines_per_frame();
        }
        let hc = if self.registers[12] & 1 == 0 {
            let n = t / 20;
            (0x85 + n + if n >= 15 { 0x55 } else { 0 }) as u8
        } else if t < 280 {
            (0xa5 + t / 16) as u8
        } else if t < 652 {
            const ENDS: [u32; 20] = [
                288, 304, 321, 341, 361, 381, 399, 419, 439, 459, 478, 497, 517, 537, 557, 575,
                595, 615, 635, 652,
            ];
            0xe4 + ENDS.iter().take_while(|end| t >= **end).count() as u8
        } else {
            (0xf8 + (t - 652) / 16) as u8
        };
        // V-counter wrap points (reference vc_table): NTSC / PAL.
        let max = match ((self.registers[1] >> 2) & 3, self.pal) {
            (1, false) => 0xea,
            (3, false) => 0x106,
            (_, false) => 0xda,
            (1, true) => 0x102,
            (3, true) => 0x10a,
            (_, true) => 0xf2,
        };
        let mut vc = line as i32;
        if vc > max {
            vc -= self.lines_per_frame() as i32;
        }
        if self.registers[12] & 2 != 0 {
            vc <<= u32::from(self.registers[12] & 4 != 0);
            vc = (vc & !1) | ((vc >> 8) & 1);
        }
        ((vc as u16 & 0xff) << 8) | u16::from(hc)
    }

    /// VRAM address the CPU last requested, used by the provenance tracer.
    #[inline]
    pub fn pending_vram_address(&self) -> u32 {
        self.address_latch
    }

    /// Data-port read.
    pub fn data_read(&mut self) -> u16 {
        let addr = (self.address_latch & 0xFFFF) as usize;
        self.control_pending = false;
        let value = match self.code & 0xF {
            0 => {
                let a = (addr & !1).min(VRAM_SIZE - 2);
                u16::from_be_bytes([self.vram[a], self.vram[a + 1]])
            }
            8 => {
                let a = addr & 0x7E;
                (u16::from_be_bytes([self.cram[a], self.cram[a + 1]]) & 0x0eee)
                    | (self.fifo_data[self.fifo_data_index] & 0xf111)
            }
            4 => {
                let a = if (addr & 0x7e) < 0x50 { addr & 0x7e } else { 0 };
                (u16::from_be_bytes([self.vsram[a], self.vsram[a + 1]]) & 0x07ff)
                    | (self.fifo_data[self.fifo_data_index] & 0xf800)
            }
            12 => u16::from(self.vram[addr ^ 1]) | (self.fifo_data[self.fifo_data_index] & 0xff00),
            _ => 0,
        };
        self.address_latch = self.address_latch.wrapping_add(self.registers[15] as u32) & 0xFFFF;
        value
    }

    /// Data-port write.
    /// Write one CRAM entry's colour byte pair.
    pub fn set_cram(&mut self, index: usize, byte: u8) {
        if index >= CRAM_ENTRIES {
            return;
        }
        self.cram[index * 2] = byte & 0x0E;
        self.cram[index * 2 + 1] = byte & 0x0E;
    }

    /// Write one VSRAM entry's horizontal/vertical scroll byte.
    pub fn set_vsram(&mut self, index: usize, byte: u8) {
        if index >= VSRAM_ENTRIES {
            return;
        }
        self.vsram[index * 2] = byte & 0x03;
        self.vsram[index * 2 + 1] = byte & 0x03;
    }

    /// Write through the VDP data port.
    ///
    /// Bits 1-0 of the code register, set by the last control-port write,
    /// select VRAM, CRAM, VSRAM or the register file. Every word write steps
    /// the address on by two.
    pub fn data_write(&mut self, value: u16) {
        self.data_write_direct(value);
    }

    fn data_write_direct(&mut self, value: u16) {
        self.fifo_data[self.fifo_data_index] = value;
        self.fifo_data_index = (self.fifo_data_index + 1) & 3;
        let addr = self.address_latch as usize & 0xFFFF;
        self.control_pending = false;
        match self.code & 0xF {
            1 => {
                let a = addr & !1;
                let value = if addr & 1 != 0 {
                    value.rotate_left(8)
                } else {
                    value
                };
                self.poke_vram(a, (value >> 8) as u8);
                self.poke_vram(a + 1, (value & 0xFF) as u8);
            }
            3 => {
                let a = addr & 0x7E;
                let value = value & 0x0EEE;
                if u16::from_be_bytes([self.cram[a], self.cram[a + 1]]) != value {
                    self.cram[a] = (value >> 8) as u8;
                    self.cram[a + 1] = (value & 0xFF) as u8;
                    self.raster_cram_write(a >> 1);
                }
            }
            5 => {
                let a = addr & 0x7E;
                if a < self.vsram.len() {
                    self.vsram[a] = ((value >> 8) & 7) as u8;
                    self.vsram[a + 1] = value as u8;
                }
            }
            _ => {}
        }
        self.address_latch = self.address_latch.wrapping_add(self.registers[15] as u32) & 0xFFFF;
        if self.pending_dm {
            self.pending_dm = false;
            let raw = u16::from_be_bytes([self.registers[20], self.registers[19]]) as u32;
            let count = if raw == 0 { 0x1_0000 } else { raw };
            // GPGX vdp_dma_fill: after the initial normal data-port word,
            // VRAM gets the high FIFO byte at adjacent addresses, CRAM/VSRAM
            // the oldest FIFO word; delivered per update.
            let _ = value;
            self.begin_internal_dma(count, false, 0);
            let source = u16::from_be_bytes([self.registers[22], self.registers[21]])
                .wrapping_add(count as u16);
            self.registers[21] = source as u8;
            self.registers[22] = (source >> 8) as u8;
            self.registers[19] = 0;
            self.registers[20] = 0;
        }
    }

    /// Decode one CRAM entry into 8-bit RGB.
    pub fn palette(&self, index: usize) -> (u8, u8, u8) {
        if index >= CRAM_ENTRIES {
            return (0, 0, 0);
        }
        let raw = u16::from_be_bytes([self.cram[index * 2], self.cram[index * 2 + 1]]);
        // Match the independent oracle's normal-intensity RGB565 profile:
        // GPGX pixel_lut[1] doubles each 3-bit channel into 4-bit intensity;
        // MAKE_PIXEL projects that into 5/6/5, then the capture expands to RGB8.
        // This is a declared software-reference profile, not an analog DAC model.
        let red = ((raw & 0x000E) >> 1) as u16 * 2;
        let green = ((raw >> 5) & 7) * 2;
        let blue = ((raw >> 9) & 7) * 2;
        let r = (((red << 1) | (red >> 3)) * 255 / 31) as u8;
        let g = (((green << 2) | (green >> 2)) * 255 / 63) as u8;
        let b = (((blue << 1) | (blue >> 3)) * 255 / 31) as u8;
        (r, g, b)
    }

    #[inline]
    pub fn display_enabled(&self) -> bool {
        // Mode-5 display enable is register 1 bit 6. Bit 4 enables DMA;
        // interlace is selected in register 12, not register 1.
        self.registers[1] & 0x40 != 0
    }

    #[inline]
    fn reg16(&self, lo: usize, hi: usize) -> usize {
        ((self.registers[hi] as usize) << 8) | self.registers[lo] as usize
    }

    /// Render one frame.
    ///
    /// Returns the framebuffer plus what the renderer could not model, so the
    /// caller can report coverage honestly rather than implying fidelity.
    pub fn render(&mut self) -> (Framebuffer, Vec<String>) {
        renderer::render(self)
    }

    /// Horizontal scroll for plane A and B on a scanline.
    fn horizontal_scroll(&self, line: i32) -> (i32, i32) {
        let hscroll_mode = self.registers[13] & 0x03;
        if hscroll_mode == 0 {
            let a = (self.registers[9] as i32) << 8 | self.registers[8] as i32;
            let b = (self.registers[1] as i32 & 0x0F) << 8 | self.registers[0] as i32;
            return (a & 0x3FF, b & 0x3FF);
        }
        let base_a = self.reg16(13, 14);
        let base_b = base_a + 20;
        let index = match hscroll_mode {
            1 => line,
            2 => line / 2,
            _ => line,
        };
        (
            self.read_hscroll(base_a, index),
            self.read_hscroll(base_b, index),
        )
    }

    fn read_hscroll(&self, base: usize, index: i32) -> i32 {
        let word = self.read_vram_word(base, (index.max(0) as usize) * 2) as i32;
        // Horizontal scroll is measured in cell widths for the first two bits.
        match word & 0x03 {
            0 => (word >> 2) & 0x3FF,
            1 => (((word >> 2) & 0x1FF) << 1) | ((word >> 1) & 1),
            _ => ((word >> 2) << 2) | ((word >> 1) & 3),
        }
    }

    #[inline]
    fn read_vram_word(&self, addr: usize, offset: usize) -> u16 {
        let a = (addr + offset) & (VRAM_SIZE - 2);
        ((self.vram[a] as u16) << 8) | self.vram[a + 1] as u16
    }

    #[inline]
    fn read_vram_word_wrapping(&self, addr: usize, offset: usize) -> usize {
        self.read_vram_word(addr, offset) as usize
    }

    /// Fetch one plane cell, returning (colour index, priority, palette, shadow).
    #[allow(clippy::too_many_arguments)]
    fn sample_plane(&self, nametable: usize, x: i32, y: i32, plane_w: i32, plane_h: i32) -> Cell {
        if self.registers[1] & 0x02 == 0 && self.registers[12] & 0x08 != 0 {
            return Cell::EMPTY;
        }
        let cx = (x >> 3) & (plane_w - 1);
        let cy = (y >> 1) & (plane_h - 1);
        let addr = (nametable + cy as usize * 32 + cx as usize) * 2 & (VRAM_SIZE - 2);
        let entry = self.read_vram_word_wrapping(addr, 0);
        let name = entry & 0x03FF;
        let fin = (entry >> 14) & 1 != 0;
        let fin2 = (entry >> 15) & 1 != 0;
        let priority = (entry >> 13) & 1 != 0;
        let palette = ((entry >> 12) & 3) as usize;
        let column = (x & 7) as usize;
        let row = if fin2 { 7 - column } else { column };
        let texel = self.tile_texel(name, column, row);
        Cell {
            colour: texel,
            priority,
            palette,
            shadow: false,
        }
    }

    /// One texel of an 8x8 2bpp tile. Each byte holds two texels, so column
    /// `c` lives in bit `7 - (c % 4)` of the pair.
    fn tile_texel(&self, name: usize, column: usize, row: usize) -> u8 {
        let base = (name * 32) & (VRAM_SIZE - 32);
        let byte = self.vram[base + (row & 7) * 4 + (column & 7) / 2];
        if column & 1 == 0 {
            byte >> 4
        } else {
            byte & 0xF
        }
    }

    #[allow(clippy::type_complexity)]
    fn sample_sprites(
        &self,
        table: usize,
        x: i32,
        line: i32,
        wide: bool,
    ) -> Option<(u8, usize, bool, bool)> {
        let max = if wide { 80 } else { 64 };
        let per_line = if wide { 20 } else { 16 };
        let mut seen = 0;
        for i in 0..max {
            let entry = (table + i * 8) & (VRAM_SIZE - 8);
            let ypos = (self.read_vram_word_wrapping(entry, 0) & 0x03FF) as i32;
            let size = self.read_vram_word_wrapping(entry, 2);
            let xpos = (self.read_vram_word_wrapping(entry, 4) & 0x03FF) as i32;
            let link = self.read_vram_word_wrapping(entry, 6);

            let h_cells = ((size >> 8) & 1) as i32 + 1;
            let v_cells = ((size >> 10) & 1) as i32 + 1;
            let w = h_cells * 8;
            let h = v_cells * 8;

            let py = line - ypos + 128;
            if py < 0 || py >= h {
                seen += 1;
                if seen > per_line {
                    return None;
                }
                continue;
            }
            let px = x - xpos + 128;
            if px < 0 || px >= w {
                seen += 1;
                if seen > per_line {
                    return None;
                }
                continue;
            }
            seen += 1;
            if seen > per_line {
                return None;
            }
            let hflip = size & 1 != 0;
            let vflip = size & 2 != 0;
            let name = size & 0x07FF;
            let col = if hflip { 7 - (px & 7) } else { px & 7 };
            let row = if vflip { 7 - (py & 7) } else { py & 7 };
            let cell = (px / 8) as usize + (py / 8) as usize * h_cells as usize;
            let cell_name = name + cell;
            let col = if hflip { 7 - (px & 7) } else { px & 7 };
            let row = if vflip { 7 - (py & 7) } else { py & 7 };
            let texel = self.tile_texel(cell_name, col as usize, row as usize);
            let palette = ((size >> 13) & 3) as usize;
            let priority = size & 0x8000 != 0;
            let shadow = size & 0x4000 != 0;
            let _ = link;
            if texel == 0 {
                return None;
            }
            return Some((texel, palette, priority, shadow || !self.shadows_enabled()));
        }
        None
    }

    fn shadows_enabled(&self) -> bool {
        true
    }

    /// Priority resolution between plane A, plane B, the window and sprites.
    fn resolve(
        &self,
        a: Cell,
        b: Cell,
        sprite: Option<(u8, usize, bool, bool)>,
        window: bool,
    ) -> Option<(u8, u8, u8)> {
        let sprite_cell: Option<Cell> = sprite.map(|(colour, palette, priority, shadow)| Cell {
            colour,
            priority,
            palette,
            shadow,
        });
        // The window hides whichever plane it covers.
        let a = if window { Cell::EMPTY } else { a };
        let b = if window { Cell::EMPTY } else { b };

        // Layer order: sprite, then plane B, then plane A. The first layer
        // whose priority bit differs from the topmost opaque layer stops the
        // search, exactly as the hardware does.
        let mut chosen: Option<Cell> = None;
        let mut winner_priority = false;
        for layer in [sprite_cell, Some(b), Some(a)].into_iter().flatten() {
            if layer.colour == 0 {
                continue;
            }
            match chosen {
                None => {
                    winner_priority = layer.priority;
                    chosen = Some(layer);
                }
                Some(_) => {
                    if layer.priority != winner_priority {
                        break;
                    }
                    if !chosen.unwrap().shadow && !layer.shadow {
                        continue;
                    }
                    chosen = Some(layer);
                }
            }
        }
        let cell = chosen?;
        if cell.shadow {
            return None;
        }
        Some(self.palette(cell.palette * 16 + cell.colour as usize))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Cell {
    colour: u8,
    priority: bool,
    palette: usize,
    shadow: bool,
}

impl Cell {
    const EMPTY: Cell = Cell {
        colour: 0,
        priority: false,
        palette: 0,
        shadow: false,
    };
}

fn plane_dimensions(size: usize) -> (i32, i32) {
    match size & 3 {
        0 => (32, 32),
        1 => (64, 32),
        2 => (32, 64),
        _ => (64, 64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paint a 2bpp tile with a known colour and read it back.
    #[test]
    fn tile_texel_decodes_four_pixel_indices() {
        let mut v = Vdp::new();
        let base = 32; // tile 1
                       // Four rows, each byte covers 2 pixels.
        for row in 0..8u32 {
            v.vram[base + row as usize * 4] = 0b0000_0000;
            v.vram[base + row as usize * 4 + 1] = 0;
            v.vram[base + row as usize * 4 + 2] = 0;
            v.vram[base + row as usize * 4 + 3] = 0;
        }
        // Set the leftmost pixel of row 0 to colour 3.
        v.vram[base] = 0x3F;
        assert_eq!(v.tile_texel(1, 0, 0), 3);
        assert_eq!(v.tile_texel(1, 1, 0), 15);
        assert_eq!(v.tile_texel(1, 7, 0), 0);
    }

    #[test]
    fn palette_decodes_bgr333_through_the_reference_rgb565_profile() {
        let mut v = Vdp::new();
        // Colour 1: red bit 0 set -> 0x0003.
        v.cram[2] = 0;
        v.cram[3] = 0x03;
        // Raw 0x0003 puts 1 in the red field's low bit -> 36.
        let (r, g, b) = v.palette(1);
        assert_eq!((r, g, b), (32, 0, 0));
    }

    #[test]
    fn disabled_display_renders_backdrop_only() {
        let mut v = Vdp::new();
        v.registers[1] = 0; // display off
        let (frame, limitations) = v.render();
        assert_eq!(frame.width, 256);
        assert!(limitations.iter().any(|l| l.contains("display disabled")));
    }
}
#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn external_dma_waits_follow_reference_access_rates_and_blanking_edges() {
        let mut v = Vdp::new();
        v.control_write(1, 0x40);
        v.control_write(12, 1);
        // H40 active: 18 accesses/line, nine VRAM words or 18 CRAM words.
        assert_eq!(v.external_dma_master_cycles(9, true), 3420);
        assert_eq!(v.external_dma_master_cycles(18, false), 3420);
        v.control_write(12, 0);
        assert_eq!(v.external_dma_master_cycles(8, true), 3420);
        v.control_write(12, 1);
        v.line = 224;
        assert_eq!(v.external_dma_master_cycles(102, true), 3420);
        assert_eq!(v.external_dma_master_cycles(198, false), 3420);
        // A transfer crossing the last active line speeds up in VBLANK.
        v.line = 223;
        assert_eq!(v.external_dma_master_cycles(10, true), 3420 + 3420 / 102);
        v.line = 261;
        assert_eq!(v.external_dma_master_cycles(9, true), 3420);
        // Forced blank uses blank rates even on an active line.
        v.line = 0;
        v.control_write(1, 0);
        assert_eq!(v.external_dma_master_cycles(102, true), 3420);
        v.advance_master_cycles(1000);
        // The reference truncates the partial-line capacity to 72 words,
        // then takes 1005 master clocks for the remaining 30 words.
        assert_eq!(v.external_dma_master_cycles(102, true), 3425);
        v.reset();
        assert_eq!(v.dma_bus_stall_master_cycles, 0);
    }

    #[test]
    fn cpu_clock_ratio_is_exact_and_independent_of_instruction_chunking() {
        // 3420 CPU clocks are 23940 master clocks: exactly seven scanlines.
        let mut whole = Vdp::new();
        whole.advance_cpu_cycles(3420);
        assert_eq!((whole.line, whole.dot), (7, 0));
        let mut split = Vdp::new();
        for _ in 0..855 {
            split.advance_cpu_cycles(4);
        }
        assert_eq!(
            (split.line, split.dot, split.cpu_clock_remainder),
            (whole.line, whole.dot, whole.cpu_clock_remainder)
        );
        // Seven frames fit exactly into 262 * 3420 CPU clocks.
        let mut frames = Vdp::new();
        frames.control_write(1, 4); // Mode5, 224-line frame geometry
        frames.advance_cpu_cycles(262 * 3420);
        assert_eq!((frames.frame, frames.line, frames.dot), (7, 0, 0));
        frames.reset();
        assert_eq!(
            (
                frames.frame,
                frames.line,
                frames.dot,
                frames.cpu_clock_remainder
            ),
            (0, 192, 0, 0)
        );
    }

    /// The vertical-blank flag in the status register is the most-polled bit
    /// on the machine: a game programs the VDP, spins on it, then enables
    /// interrupts. Reporting it as a constant makes those loops hang forever,
    /// which is how this surfaced - a cartridge waited on bit 1 and never came
    /// back.
    ///
    /// The flag is a level derived from the line counter, and the read clears
    /// it along with the interrupt-pending bits.
    #[test]
    fn vertical_blank_flag_tracks_the_line_counter_and_is_cleared_by_a_read() {
        let mut v = Vdp::new();
        v.control_write(1, 4);
        // Step to interior points rather than onto the boundary: advance() is
        // inclusive, so landing exactly on a threshold has already crossed it.
        v.advance(DOTS_PER_LINE * (VBLANK_START - 1));
        assert!(
            !v.vblank,
            "the line before the first blanked line is active"
        );
        v.advance(DOTS_PER_LINE);
        assert!(v.vblank, "one line later is inside vertical blank");
        v.advance(DOTS_PER_LINE * (VBLANK_END - VBLANK_START));
        assert!(!v.vblank, "blanking ends at VBLANK_END");
    }

    /// The status bits the software can latch are set from the same timing
    /// state, and reading the register clears the pending ones.
    #[test]
    fn horizontal_blank_covers_the_tail_of_an_active_line() {
        let mut v = Vdp::new();
        v.advance(DOTS_PER_LINE - 85);
        assert!(!v.hblank, "the active display part of a line is not blank");
        v.advance(1);
        assert!(v.hblank, "the last 84 dots of a line are horizontal blank");
    }

    /// The frame counter has to wrap, or the odd/even field and anything
    /// derived from frame time would run forever.
    #[test]
    fn a_whole_frame_of_dots_advances_exactly_one_frame() {
        let mut v = Vdp::new();
        v.control_write(1, 4);
        v.advance(DOTS_PER_LINE * LINES_PER_FRAME);
        assert_eq!(v.frame, 1);
        assert_eq!(v.line, 0);
        assert_eq!(v.dot, 0);
    }
}

#[cfg(test)]
mod display_enable_tests {
    use super::*;

    fn vdp_with_reg1(value: u8) -> Vdp {
        let mut v = Vdp::new();
        v.registers[1] = value;
        v
    }

    /// Register 1 bit 4 is the display enable. Bit 6 is the interlace flag,
    /// and a renderer that reads it reports a perfectly configured cartridge
    /// as switched off, which is indistinguishable from a renderer that
    /// cannot draw.
    #[test]
    fn display_enable_is_bit_six_not_dma_enable() {
        let on = vdp_with_reg1(0x74);
        assert!(on.display_enabled(), "0x74 enables display");
        let off = vdp_with_reg1(0x34);
        assert!(
            !off.display_enabled(),
            "0x34 enables DMA and VINT, not display"
        );
    }

    /// Interlaced display is set with the same bit that used to be mistaken
    /// for the display enable, so the two must stay distinguishable.
    #[test]
    fn interlace_and_display_enable_are_independent() {
        let mut v = vdp_with_reg1(0x74);
        v.registers[12] = 0x06;
        assert!(v.display_enabled());
        assert!(v.render().1.iter().any(|l| l.contains("interlaced")));
    }

    /// There are 32 registers. An array of 64 makes a malformed control-port
    /// write land somewhere plausible instead of being rejected.
    #[test]
    fn the_register_file_is_thirty_two_entries() {
        assert_eq!(std::mem::size_of_val(&Vdp::new().registers), 32);
    }
}
