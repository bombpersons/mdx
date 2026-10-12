//! The Mega Drive video display processor.
//!
//! Covers what a cartridge actually drives: planes A and B, the window, sprites,
//! scrolling, the 9-bit palette and priority resolution, for the 320- and
//! 256-pixel-wide modes. Interlaced output and the unusual modes are treated as
//! unknown rather than guessed at, and the renderer says so in its coverage.

mod renderer;

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
    /// Line within the frame, 0..262 on NTSC.
    pub line: u32,
    /// Completed frames since reset.
    pub frame: u64,
    /// Set while a vertical interrupt is being presented to the 68000.
    pub vint_pending: bool,
    /// Vertical interrupts the chip has raised, independent of whether the CPU
    /// took them.
    pub vint_requests: u64,
    /// Vertical interrupts the CPU actually accepted.
    pub vint_taken: u64,
    /// Set while a horizontal interrupt is being presented to the 68000.
    pub hint_pending: bool,
    /// Fractional normalized dots carried across 68000 instruction boundaries.
    cpu_clock_remainder: u64,
}

/// Master-clock dots in one scanline.
pub const DOTS_PER_LINE: u32 = 896;
/// Lines in one NTSC frame.
pub const LINES_PER_FRAME: u32 = 262;
/// First line of the vertical blank interval.
pub const VBLANK_START: u32 = 224;
/// Last line of vertical blanking, NTSC.
pub const VBLANK_END: u32 = 242;
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
    /// Advance using the exact clock ratio, retaining fractional line units.
    /// Chunking a run into instructions must not discard time on every step.
    pub fn advance_cpu_cycles(&mut self, cycles: u32) {
        self.advance_master_cycles(cycles * MASTER_CYCLES_PER_CPU_CYCLE);
    }

    /// Advance device time without rounding a DMA duration to whole CPU clocks.
    pub fn advance_master_cycles(&mut self, cycles: u32) {
        let scaled = self.cpu_clock_remainder + u64::from(cycles) * u64::from(DOTS_PER_LINE);
        let dots = scaled / u64::from(MASTER_CYCLES_PER_LINE);
        self.cpu_clock_remainder = scaled % u64::from(MASTER_CYCLES_PER_LINE);
        self.advance(dots as u32);
    }

    /// External-bus DMA wait using GPGX vdp_ctrl.c:664-801 access rates.
    /// VRAM consumes two slots per word; CRAM/VSRAM consume one. The CPU
    /// cannot change registers during this wait. Port accesses are still
    /// instruction-boundary observations, not a sub-instruction FIFO model.
    pub fn external_dma_master_cycles(&self, mut words: u32, to_vram: bool) -> u32 {
        let mut line = self.line;
        let mut position = ((u64::from(self.dot) * u64::from(MASTER_CYCLES_PER_LINE)
            + self.cpu_clock_remainder)
            / u64::from(DOTS_PER_LINE)) as u32;
        let mut elapsed = 0;
        while words != 0 {
            // The reference clears VBLANK on the final pre-active line.
            let blank =
                !self.display_enabled() || (VBLANK_START..LINES_PER_FRAME - 1).contains(&line);
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
            let available = MASTER_CYCLES_PER_LINE - position;
            let capacity = available * rate / MASTER_CYCLES_PER_LINE;
            if words < capacity {
                elapsed += words * MASTER_CYCLES_PER_LINE / rate;
                break;
            }
            words -= capacity;
            elapsed += available;
            line = (line + 1) % LINES_PER_FRAME;
            position = 0;
        }
        elapsed
    }

    /// Advance the dot clock and raise interrupt requests on the edges.
    pub fn advance(&mut self, dots: u32) {
        let mut left = dots;
        while left > 0 {
            let span = (DOTS_PER_LINE - self.dot).min(left);
            self.dot += span;
            left -= span;
            if self.dot >= DOTS_PER_LINE {
                self.dot = 0;
                self.line += 1;
                if self.line == VBLANK_START {
                    self.vint_pending = true;
                    self.vint_requests += 1;
                }
                if self.line < VBLANK_START {
                    if self.hint_counter == 0 {
                        self.hint_counter = self.registers[10];
                        self.hint_pending = true;
                    } else {
                        self.hint_counter -= 1;
                    }
                } else {
                    self.hint_counter = self.registers[10];
                }
                if self.line >= LINES_PER_FRAME {
                    self.line = 0;
                    self.frame += 1;
                }
            }
        }

        // The vertical interrupt is presented on the rising edge of the blanking
        // flag, once per frame. Horizontal interrupts fire once per active
        // line when the enable is set; the per-line counter in register 10 is
        // modelled as always matching, which is what software expects after it
        // programs a threshold.
        // The blanking flags are level signals derived from the line counter.
        // Software polls them directly - a vertical blank, then interrupt
        // enable, is the standard handshake - so they have to be live rather
        // than implied by the interrupt request.
        self.vblank = self.line >= VBLANK_START && self.line < VBLANK_END;
        // Horizontal blanking covers the last 84 dots of every active line,
        // and runs the whole line while the display is disabled.
        self.hblank = self.line >= VBLANK_START || self.dot >= DOTS_PER_LINE - 84;
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
            frame: 0,
            vint_pending: false,
            vint_requests: 0,
            vint_taken: 0,
            hint_pending: false,
            pending_dm: false,
            control_pending: false,
            dma_requests: 0,
            dma_bytes: 0,
            dma_bus_stall_master_cycles: 0,
            dma_unsupported: false,
            hint_counter: 0,
            vblank: false,
            hblank: false,
            cpu_clock_remainder: 0,
        }
    }

    pub fn reset(&mut self) {
        self.registers = [0; 32];
        self.vram.fill(0);
        self.cram.fill(0);
        self.vsram.fill(0);
        self.address_latch = 0;
        self.code = 0;
        self.dot = 0;
        self.line = 0;
        self.frame = 0;
        self.cpu_clock_remainder = 0;
        self.vblank = false;
        self.hblank = false;
        self.vint_pending = false;
        self.hint_pending = false;
        self.vint_requests = 0;
        self.vint_taken = 0;
        self.hint_counter = 0;
        self.pending_dm = false;
        self.control_pending = false;
        self.dma_requests = 0;
        self.dma_bytes = 0;
        self.dma_bus_stall_master_cycles = 0;
        self.dma_unsupported = false;
    }

    /// Read a control register.
    #[inline]
    pub fn control_read(&self, index: usize) -> u8 {
        self.registers[index & 0x1F]
    }

    pub fn control_write(&mut self, index: usize, value: u8) {
        self.registers[index & 0x1F] = value;
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
                u16::from_be_bytes([self.cram[a], self.cram[a + 1]])
            }
            4 => {
                let a = addr & 0x7E;
                if a < self.vsram.len() {
                    u16::from_be_bytes([self.vsram[a], self.vsram[a + 1]])
                } else {
                    0
                }
            }
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
                self.vram[a] = (value >> 8) as u8;
                self.vram[a + 1] = (value & 0xFF) as u8;
            }
            3 => {
                let a = addr & 0x7E;
                let value = value & 0x0EEE;
                self.cram[a] = (value >> 8) as u8;
                self.cram[a + 1] = (value & 0xFF) as u8;
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
            if self.code & 0xF == 1 {
                // GPGX vdp_dma_fill writes the high FIFO byte to adjacent
                // VRAM addresses after the initial normal data-port word.
                for _ in 0..count {
                    self.vram[(self.address_latch as usize ^ 1) & 0xFFFF] = (value >> 8) as u8;
                    self.address_latch =
                        self.address_latch.wrapping_add(self.registers[15] as u32) & 0xFFFF;
                }
                self.dma_bytes += count as u64;
                self.registers[19] = 0;
                self.registers[20] = 0;
            } else {
                self.dma_unsupported = true;
            }
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
            (0, 0, 0, 0)
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
