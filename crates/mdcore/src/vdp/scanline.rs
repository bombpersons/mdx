//! Per-scanline mode-5 renderer following GPGX `vdp_render.c` (non-ALT
//! renderer, the libretro default): each active line is drawn when the VDP
//! reaches it, from the VRAM/VSRAM/register state of that moment, and colours
//! are resolved from CRAM at the same instant. That captures raster effects
//! (mid-frame scroll, palette, plane-base and display changes), the internal
//! sprite-attribute cache, sprite limits/masking, shadow/highlight and the
//! window-plane rules, plus the sprite collision/overflow status bits.
use super::{Framebuffer, Vdp};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const MAX_WIDTH: usize = 320;
pub const MAX_HEIGHT: usize = 240;
/// Screen code for a pixel GPGX leaves black (mode 4, invalid intensity).
const BLACK: u16 = 3 << 9;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    attr: u16,
    xpos: u16,
    ypos: u16,
    size: u8,
}

/// Renderer state that persists between lines (and in save states).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Raster {
    /// Internal copy of the sprite attribute table: only VRAM writes that
    /// land in the current table update it, so moving the table base does not.
    sat: Vec<u8>,
    /// The cache was initialised (states written before it existed copy VRAM).
    sat_valid: bool,
    objects: [Vec<Object>; 2],
    spr_ovr: bool,
    /// Pending SOVR (0x40) / SCOL (0x20) status bits, cleared by status reads.
    pub sprite_status: u8,
    /// Active width latched like GPGX `bitmap.viewport.w`.
    pub width: u16,
    pub width_pending: bool,
    /// Dimensions of the last completed frame.
    pub output_width: u16,
    pub output_height: u16,
    /// Per-pixel output codes: `intensity << 9 | BGR333`, intensity 0 shadow,
    /// 1 normal, 2 highlight, 3 black. Row stride is `MAX_WIDTH`.
    screen: Vec<u16>,
    /// Pixel bytes of the most recently rendered line, kept for HBLANK remaps.
    line: Vec<u8>,
}

impl Default for Raster {
    fn default() -> Self {
        Self {
            sat: vec![0; 0x400],
            sat_valid: false,
            objects: [Vec::new(), Vec::new()],
            spr_ovr: false,
            sprite_status: 0,
            width: 256,
            width_pending: false,
            output_width: 256,
            output_height: 224,
            screen: vec![BLACK; MAX_WIDTH * MAX_HEIGHT],
            line: vec![0x40; MAX_WIDTH],
        }
    }
}

impl Raster {
    pub fn reset(&mut self) {
        *self = Self { sat_valid: true, output_height: 192, ..Self::default() };
    }

    /// The internal sprite-attribute cache (Y, size and link of each entry).
    pub fn sat_cache(&self) -> &[u8] {
        &self.sat
    }

    pub(crate) fn valid(&self) -> bool {
        self.sat.len() == 0x400
            && self.screen.len() == MAX_WIDTH * MAX_HEIGHT
            && self.line.len() == MAX_WIDTH
            && matches!(self.width, 256 | 320)
            && matches!(self.output_width, 256 | 320)
            && usize::from(self.output_height) <= MAX_HEIGHT
            && self.objects.iter().all(|o| o.len() <= 20)
    }
}

const LUT_BG: usize = 0;
const LUT_BGOBJ: usize = 1;
const LUT_BG_STE: usize = 2;
const LUT_OBJ: usize = 3;
const LUT_BGOBJ_STE: usize = 4;

fn lut_bg(bx: u32, ax: u32, ste: bool) -> u8 {
    let (bf, bp, b) = (bx & 0x7F, bx & 0x40, bx & 0x0F);
    let (af, ap, a) = (ax & 0x7F, ax & 0x40, ax & 0x0F);
    let mut c = if ap != 0 {
        if a != 0 { af } else { bf }
    } else if bp != 0 {
        if b != 0 { bf } else { af }
    } else if a != 0 {
        af
    } else {
        bf
    };
    if ste {
        // Half intensity when both pixels are low priority.
        c |= (ap | bp) << 1;
    }
    if c & 0x0F == 0 {
        c &= 0x80;
    }
    c as u8
}

fn lut_obj(bx: u32, sx: u32) -> u8 {
    if sx & 0x0F == 0 {
        return bx as u8;
    }
    let mut c = if bx & 0x80 != 0 { bx & 0x7F } else { sx & 0x7F };
    if c & 0x0F == 0 {
        c &= 0xC0;
    }
    (c | 0x80) as u8
}

fn lut_bgobj(bx: u32, sx: u32) -> u8 {
    let (bf, bs, bp, b) = (bx & 0x3F, bx & 0x80, bx & 0x40, bx & 0x0F);
    let (sf, sp, s) = (sx & 0x3F, sx & 0x40, sx & 0x0F);
    if s == 0 || bs != 0 {
        return bx as u8;
    }
    let mut c = if sp != 0 || bp == 0 || b == 0 { sf } else { bf };
    if c & 0x0F == 0 {
        c &= 0x80;
    }
    (c | 0x80) as u8
}

fn lut_bgobj_ste(bx: u32, sx: u32) -> u8 {
    let (bf, bp, b, bi) = (bx & 0x3F, bx & 0x40, bx & 0x0F, (bx & 0x80) >> 1);
    let (sf, sp, s) = (sx & 0x3F, sx & 0x40, sx & 0x0F);
    let si = sp | bi;
    let sprite = || {
        if sf & 0x3E == 0x3E {
            if sf & 1 != 0 {
                bf
            } else if bx & 0x80 != 0 {
                bf | 0x80
            } else {
                bf | 0x40
            }
        } else if matches!(sf, 0x0E | 0x1E | 0x2E) {
            sf | 0x40
        } else {
            sf | si
        }
    };
    let mut c = if sp != 0 {
        if s != 0 { sprite() } else { bf | bi }
    } else if bp != 0 && b != 0 {
        bf | bi
    } else if s != 0 {
        sprite()
    } else {
        bf | bi
    };
    if c & 0x0F == 0 {
        c &= 0xC0;
    }
    c as u8
}

fn luts() -> &'static [Vec<u8>; 5] {
    static LUTS: OnceLock<[Vec<u8>; 5]> = OnceLock::new();
    LUTS.get_or_init(|| {
        let mut tables: [Vec<u8>; 5] = std::array::from_fn(|_| vec![0; 0x10000]);
        for bx in 0..0x100u32 {
            for ax in 0..0x100u32 {
                let i = (bx << 8 | ax) as usize;
                tables[LUT_BG][i] = lut_bg(bx, ax, false);
                tables[LUT_BGOBJ][i] = lut_bgobj(bx, ax);
                tables[LUT_BG_STE][i] = lut_bg(bx, ax, true);
                tables[LUT_OBJ][i] = lut_obj(bx, ax);
                tables[LUT_BGOBJ_STE][i] = lut_bgobj_ste(bx, ax);
            }
        }
        tables
    })
}

/// Sprite pattern offset for (column, row, height, width, hflip, vflip).
fn name_offset(index: usize) -> u16 {
    let (mut col, mut row) = (index & 3, (index >> 2) & 3);
    let (height, width) = ((index >> 4) & 3, (index >> 6) & 3);
    if index & 0x100 != 0 {
        col = width.wrapping_sub(col);
    }
    if index & 0x200 != 0 {
        row = height.wrapping_sub(row);
    }
    (row + col * (height + 1)) as u16
}

const HSCROLL_MASK: [u32; 4] = [0x00, 0x07, 0xF8, 0xFF];
const PF_SHIFT: [u32; 4] = [6, 7, 0, 8];
const PF_COL_MASK: [u32; 4] = [0x0F, 0x1F, 0x0F, 0x3F];
const PF_ROW_MASK: [u32; 4] = [0x0FF, 0x1FF, 0x2FF, 0x3FF];

#[derive(Clone, Copy, Default)]
struct Clip {
    left: usize,
    right: usize,
    enable: bool,
}

/// GPGX `window_clip`: index 0 is plane A, 1 the window.
fn window_clip(reg17: u8, h40: bool) -> [Clip; 2] {
    let hp = usize::from(reg17 & 0x1F);
    let hf = usize::from(reg17 >> 7);
    let (a, w) = (hf, hf ^ 1);
    let sw = if h40 { 20 } else { 16 };
    let mut clip = [Clip::default(); 2];
    if hp == 0 {
        clip[a] = Clip { left: 0, right: sw, enable: true };
    } else if hp > sw {
        clip[w] = Clip { left: 0, right: sw, enable: true };
    } else {
        clip[w] = Clip { left: 0, right: hp, enable: true };
        clip[a] = Clip { left: hp, right: sw, enable: true };
    }
    clip
}

/// Line buffers sized like GPGX's: visible pixel x is at 0x20 + x.
struct Lines {
    bg: [u8; 0x200],
    fg: [u8; 0x200],
}

impl Vdp {
    fn vram_word(&self, address: u32) -> u32 {
        let a = address as usize & 0xFFFE;
        u32::from(u16::from_be_bytes([self.vram[a], self.vram[a + 1]]))
    }

    fn vsram_word(&self, index: usize) -> u32 {
        let a = (index * 2) % self.vsram.len();
        u32::from(u16::from_be_bytes([self.vsram[a], self.vsram[a + 1]]))
    }

    fn sat_base(&self) -> (u32, u32, u32) {
        if self.registers[12] & 1 != 0 {
            ((u32::from(self.registers[5]) << 9) & 0xFC00, 0xFC00, 0x03FF)
        } else {
            ((u32::from(self.registers[5]) << 9) & 0xFE00, 0xFE00, 0x01FF)
        }
    }

    /// Write one VRAM byte, mirroring writes into the internal SAT cache.
    pub(crate) fn poke_vram(&mut self, address: usize, value: u8) {
        let address = address & 0xFFFF;
        self.ensure_sat();
        let (satb, base_mask, addr_mask) = self.sat_base();
        if address as u32 & base_mask == satb {
            self.raster.sat[address & addr_mask as usize] = value;
        }
        self.vram[address] = value;
    }

    fn ensure_sat(&mut self) {
        if !self.raster.sat_valid {
            let (satb, _, addr_mask) = self.sat_base();
            for i in 0..=addr_mask as usize {
                self.raster.sat[i] = self.vram[(satb as usize + i) & 0xFFFF];
            }
            self.raster.sat_valid = true;
        }
    }

    /// Eight pattern pixels of one tile row with flips applied; `attr` holds
    /// the name and flip bits (VH NNNNNNNNNNN).
    fn tile_row(&self, attr: u32, row: u32) -> [u8; 8] {
        let row = if attr & 0x1000 != 0 { 7 - row } else { row };
        let base = ((attr & 0x7FF) << 5 | row << 2) as usize;
        let mut out = [0; 8];
        for (x, pixel) in out.iter_mut().enumerate() {
            let x = if attr & 0x800 != 0 { 7 - x } else { x };
            let byte = self.vram[base + x / 2];
            *pixel = if x & 1 == 0 { byte >> 4 } else { byte & 0x0F };
        }
        out
    }

    /// GPGX DRAW_COLUMN: two consecutive name-table words, 16 pixels.
    fn draw_column(&self, buffer: &mut [u8; 0x200], dst: usize, pair: u32, row: u32) {
        for (half, attr) in [(0, self.vram_word(pair)), (8, self.vram_word(pair + 2))] {
            let atex = ((attr >> 9) & 0x70) as u8;
            for (i, pixel) in self.tile_row(attr, row).iter().enumerate() {
                buffer[dst + half + i] = pixel | atex;
            }
        }
    }

    fn render_bg(&self, line: u32, lines: &mut Lines) {
        let r = &self.registers;
        let h40 = r[12] & 1 != 0;
        let width = usize::from(self.raster.width);
        let hscb = (u32::from(r[13]) << 10) & 0xFC00;
        let xs = hscb + ((line & HSCROLL_MASK[usize::from(r[11] & 3)]) << 2);
        let (xa, xb) = (self.vram_word(xs), self.vram_word(xs + 2));
        let column_scroll = r[11] & 4 != 0;
        let col_mask = PF_COL_MASK[usize::from(r[16] & 3)];
        let row_mask = PF_ROW_MASK[usize::from((r[16] >> 4) & 3)];
        let pf_shift = PF_SHIFT[usize::from(r[16] & 3)];
        let ntab = (u32::from(r[2]) << 10) & 0xE000;
        let ntbb = (u32::from(r[4]) << 13) & 0xE000;
        let ntwb = (u32::from(r[3]) << 10) & if h40 { 0xF000 } else { 0xF800 };
        // Left partially shown column: in H32 no vertical scroll, in H40 the
        // AND of both planes' column-19 values (column scroll mode only).
        let edge_scroll = if h40 { self.vsram_word(38) & self.vsram_word(39) } else { 0 };
        let yscroll = |column: Option<usize>, plane: usize| -> u32 {
            match (column_scroll, column) {
                (false, _) => self.vsram_word(plane),
                (true, Some(column)) => self.vsram_word(column * 2 + plane),
                (true, None) => edge_scroll,
            }
        };
        let row_address = |base: u32, v_line: u32| base + (((v_line >> 3) << pf_shift) & 0x1FC0);

        // Plane B.
        let end = width >> 4;
        let shift = (xb & 0x0F) as usize;
        let mut index = (col_mask + 1).wrapping_sub((xb >> 4) & col_mask);
        if shift != 0 {
            let v_line = (line + yscroll(None, 1)) & row_mask;
            let nt = row_address(ntbb, v_line);
            let pair = nt + (index.wrapping_sub(1) & col_mask) * 4;
            self.draw_column(&mut lines.bg, 0x10 + shift, pair, v_line & 7);
        }
        let dst = if shift != 0 { 0x20 + shift } else { 0x20 };
        for column in 0..end {
            let v_line = (line + yscroll(Some(column), 1)) & row_mask;
            let nt = row_address(ntbb, v_line);
            self.draw_column(&mut lines.bg, dst + column * 16, nt + (index & col_mask) * 4, v_line & 7);
            index = index.wrapping_add(1);
        }

        // Plane A and window.
        let split = u32::from(r[18] & 0x1F) << 3;
        let below = (r[18] >> 7) & 1 != 0;
        let clip = window_clip(r[17], h40);
        let (a_enable, w_enable) = if below == (line >= split) {
            (false, true)
        } else {
            (clip[0].enable, clip[1].enable)
        };
        let (mut start, mut stop) = (0, end);
        if a_enable {
            start = clip[0].left;
            stop = clip[0].right;
            let shift = (xa & 0x0F) as usize;
            let mut index = (col_mask + start as u32 + 1).wrapping_sub((xa >> 4) & col_mask);
            if shift != 0 {
                let v_line = (line + yscroll(None, 0)) & row_mask;
                let nt = row_address(ntab, v_line);
                // Window bug: a column after the window repeats the first one.
                let column = if start != 0 { index } else { index.wrapping_sub(1) };
                let pair = nt + (column & col_mask) * 4;
                self.draw_column(&mut lines.fg, 0x10 + shift + (start << 4), pair, v_line & 7);
            }
            let dst = if shift != 0 { 0x20 + shift } else { 0x20 };
            for column in start..stop {
                let v_line = (line + yscroll(Some(column), 0)) & row_mask;
                let nt = row_address(ntab, v_line);
                self.draw_column(&mut lines.fg, dst + column * 16, nt + (index & col_mask) * 4, v_line & 7);
                index = index.wrapping_add(1);
            }
            start = clip[1].left;
            stop = clip[1].right;
        }
        if w_enable {
            let nt = ntwb | ((line >> 3) << (6 + u32::from(h40)));
            for column in start..stop {
                let pair = (nt + column as u32 * 4) & 0xFFFF;
                self.draw_column(&mut lines.fg, 0x20 + column * 16, pair, line & 7);
            }
        }
        let table = &luts()[if r[12] & 8 != 0 { LUT_BG_STE } else { LUT_BG }];
        for x in 0x20..0x20 + width {
            lines.bg[x] = table[usize::from(lines.bg[x]) << 8 | usize::from(lines.fg[x])];
        }
    }

    fn render_obj(&mut self, line: u32, lines: &mut Lines, max_pixels: usize) {
        let ste = self.registers[12] & 8 != 0;
        let width = i32::from(self.raster.width);
        if ste {
            lines.fg.fill(0);
        }
        let table = &luts()[if ste { LUT_OBJ } else { LUT_BGOBJ }];
        let objects = std::mem::take(&mut self.raster.objects[(line & 1) as usize]);
        let mut pixel_count = 0;
        let mut masked = false;
        let mut limited = false;
        for object in &objects {
            if object.xpos != 0 {
                self.raster.spr_ovr = true;
            } else if self.raster.spr_ovr {
                masked = true;
            }
            let xpos = i32::from(object.xpos) - 0x80;
            let size = u32::from(object.size);
            let mut sprite_width = 8 + ((size & 0x0C) << 1) as usize;
            pixel_count += sprite_width;
            if xpos + sprite_width as i32 > 0 && xpos < width && !masked {
                let attr = u32::from(object.attr);
                let v_line = u32::from(object.ypos);
                let atex = ((attr >> 9) & 0x70) as u8;
                let name = attr & 0x07FF;
                let flips = attr & 0x1800;
                let lut_base = ((flips >> 3) & 0x300) as usize | (size << 4) as usize | ((v_line & 0x18) >> 1) as usize;
                if pixel_count > max_pixels {
                    sprite_width -= pixel_count - max_pixels;
                }
                let buffer = if ste { &mut lines.fg } else { &mut lines.bg };
                for column in 0..sprite_width >> 3 {
                    let tile = flips | ((name + u32::from(name_offset(lut_base + column))) & 0x07FF);
                    let at = (0x20 + xpos + column as i32 * 8) as usize;
                    for (i, pixel) in self.tile_row(tile, v_line & 7).into_iter().enumerate() {
                        if pixel != 0 {
                            let below = buffer[at + i];
                            buffer[at + i] = table[usize::from(below) << 8 | usize::from(pixel | atex)];
                            if below & 0x80 != 0 {
                                self.raster.sprite_status |= 0x20;
                            }
                        }
                    }
                }
            }
            if pixel_count >= max_pixels {
                // Masking applies to the next line if the full width was used.
                self.raster.spr_ovr = pixel_count >= width as usize;
                limited = true;
                break;
            }
        }
        if !limited {
            self.raster.spr_ovr = false;
        }
        self.raster.objects[(line & 1) as usize] = objects;
        if ste {
            let table = &luts()[LUT_BGOBJ_STE];
            for x in 0x20..0x20 + width as usize {
                lines.bg[x] = table[usize::from(lines.bg[x]) << 8 | usize::from(lines.fg[x])];
            }
        }
    }

    /// Build the sprite list for `line + 1` (GPGX parse_satb_m5; -1 parses line 0).
    pub(super) fn parse_satb(&mut self, line: i32, max_pixels: usize) {
        self.ensure_sat();
        let width = usize::from(self.raster.width);
        let max = width >> 4;
        let mut total = max_pixels >> 2;
        let (satb, _, _) = self.sat_base();
        let slot = ((line + 1) & 1) as usize;
        let mut objects = std::mem::take(&mut self.raster.objects[slot]);
        objects.clear();
        let line = line + 0x81;
        let sat = |word: usize| u32::from(u16::from_be_bytes([self.raster.sat[word * 2 & 0x3FF], self.raster.sat[(word * 2 & 0x3FF) + 1]]));
        let mut link = 0usize;
        loop {
            let ypos = (sat(link) & 0x1FF) as i32;
            if line >= ypos {
                let size = sat(link + 1) >> 8;
                let height = 8 + ((size & 3) << 3) as i32;
                let offset = line - ypos;
                if offset < height {
                    if objects.len() == max {
                        self.raster.sprite_status |= 0x40;
                        break;
                    }
                    objects.push(Object {
                        attr: self.vram_word(satb + (link as u32 + 2) * 2) as u16,
                        xpos: (self.vram_word(satb + (link as u32 + 3) * 2) & 0x1FF) as u16,
                        ypos: offset as u16,
                        size: (size & 0x0F) as u8,
                    });
                }
            }
            link = ((sat(link + 1) & 0x7F) << 2) as usize;
            total -= 1;
            if link == 0 || link >= width || total == 0 {
                break;
            }
        }
        self.raster.objects[slot] = objects;
    }

    fn max_sprite_pixels(&self) -> usize {
        if self.registers[12] & 1 != 0 { 320 } else { 256 }
    }

    /// GPGX render_line for an active line.
    pub(super) fn render_line(&mut self, line: u32) {
        self.render_line_limited(line, self.max_sprite_pixels());
    }

    pub(super) fn render_line_limited(&mut self, line: u32, max_pixels: usize) {
        if line as usize >= MAX_HEIGHT {
            return;
        }
        let width = usize::from(self.raster.width);
        let mut lines = Lines { bg: [0; 0x200], fg: [0; 0x200] };
        if self.display_enabled() && self.registers[1] & 4 != 0 {
            self.render_bg(line, &mut lines);
            self.render_obj(line, &mut lines, max_pixels);
            if self.registers[0] & 0x20 != 0 {
                lines.bg[0x20..0x28].fill(0x40);
            }
            if line + 1 < self.visible_lines {
                self.parse_satb(line as i32, max_pixels);
            }
        } else {
            lines.bg[0x20..0x20 + width].fill(0x40);
        }
        self.raster.line[..width].copy_from_slice(&lines.bg[0x20..0x20 + width]);
        self.remap_line(line);
    }

    /// GPGX blank_line: backdrop over `count` pixels from `offset`, then remap.
    pub(super) fn blank_line(&mut self, line: u32, offset: usize, count: usize) {
        let end = (offset + count).min(MAX_WIDTH);
        self.raster.line[offset.min(end)..end].fill(0x40);
        self.remap_line(line);
    }

    /// Resolve the current line's pixel bytes through the present CRAM.
    pub(super) fn remap_line(&mut self, line: u32) {
        let line = line as usize;
        if line >= MAX_HEIGHT {
            return;
        }
        let width = usize::from(self.raster.width);
        let mode5 = self.registers[1] & 4 != 0;
        let ste = self.registers[12] & 8 != 0;
        let palette_mask = if self.registers[0] & 4 != 0 { 0x1FF } else { 0x49 };
        let border = usize::from(self.registers[7] & 0x3F);
        for x in 0..width {
            let pixel = self.raster.line[x];
            let code = if !mode5 || (ste && pixel >> 6 == 3) {
                BLACK
            } else {
                let index = match usize::from(pixel & 0x3F) {
                    0 => border,
                    index => index,
                };
                let raw = u16::from_be_bytes([self.cram[index * 2], self.cram[index * 2 + 1]]);
                let bgr = ((raw & 0xE00) >> 3 | (raw & 0xE0) >> 2 | (raw & 0x0E) >> 1) & palette_mask;
                let level = if ste { u16::from(pixel >> 6) } else { 1 };
                level << 9 | bgr
            };
            self.raster.screen[line * MAX_WIDTH + x] = code;
        }
    }

    /// Latch output geometry at the frame edge and apply a deferred width.
    pub(super) fn raster_frame_end(&mut self, completed_lines: u32) {
        self.raster.output_width = self.raster.width;
        self.raster.output_height = completed_lines.min(MAX_HEIGHT as u32) as u16;
        if self.raster.width_pending {
            self.raster.width_pending = false;
            self.raster.width = self.max_sprite_pixels() as u16;
        }
    }

    /// Register side effects GPGX applies to the current line (vdp_reg_w).
    pub(super) fn raster_register_write(&mut self, index: usize, old: u8, value: u8) {
        let active = self.line < self.visible_lines;
        let offset = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896;
        let early = offset <= 860;
        match index {
            1 if (old ^ value) & 0x40 != 0 && active => {
                if early {
                    let limit = if value & 0x40 != 0 && offset > 360 { 128 } else { self.max_sprite_pixels() };
                    self.render_line_limited(self.line, limit);
                } else {
                    let dot = if self.registers[12] & 1 != 0 { 8 } else { 10 };
                    let start = ((offset - 860) / dot + 16) as usize;
                    let width = usize::from(self.raster.width);
                    if start < width {
                        if value & 0x40 != 0 {
                            self.render_line(self.line);
                            self.blank_line(self.line, 0, start);
                        } else {
                            self.blank_line(self.line, start, width - start);
                        }
                    }
                }
            }
            2..=4 if active && self.display_enabled() && early => self.render_line(self.line),
            5 if active && self.display_enabled() && early => {
                self.parse_satb(self.line as i32, self.max_sprite_pixels())
            }
            7 if (old ^ value) & 0x3F != 0 && active && early => self.remap_line(self.line),
            12 if (old ^ value) & 1 != 0 => {
                if !active {
                    self.raster.width = self.max_sprite_pixels() as u16;
                } else if self.line <= 1 {
                    self.raster.width = self.max_sprite_pixels() as u16;
                    let line = self.line;
                    self.render_line(0);
                    if line == 1 {
                        self.render_line(1);
                    }
                } else {
                    self.raster.width_pending = true;
                }
            }
            _ => {}
        }
    }

    /// A CRAM write during the current line's HBLANK remaps that line.
    pub(super) fn raster_cram_write(&mut self, index: usize) {
        let offset = (u64::from(self.dot) * 3420 + self.cpu_clock_remainder) / 896;
        if self.line < self.visible_lines
            && offset <= 860
            && (self.display_enabled() || index == usize::from(self.registers[7] & 0x3F))
        {
            self.remap_line(self.line);
        }
    }

    /// The scanline-rendered frame (last completed frame plus any lines of the
    /// current one already drawn).
    pub fn frame(&self) -> (Framebuffer, Vec<String>) {
        let width = usize::from(self.raster.output_width);
        let height = usize::from(self.raster.output_height);
        let mut frame = Framebuffer::new(width, height);
        for y in 0..height {
            for x in 0..width {
                frame.set(x, y, decode(self.raster.screen[y * MAX_WIDTH + x]));
            }
        }
        let mut limitations = Vec::new();
        if self.registers[12] & 6 == 6 {
            limitations.push("interlace mode 2 (double resolution) is rendered single-field".into());
        }
        if self.registers[1] & 4 == 0 {
            limitations.push("mode 4 graphics are not rendered".into());
        }
        if self.dma_unsupported {
            limitations.push("unsupported VDP DMA destination/mode".into());
        }
        (frame, limitations)
    }
}

/// Screen code to RGB8 through the reference RGB565 profile.
fn decode(code: u16) -> (u8, u8, u8) {
    let level = code >> 9;
    if level == 3 {
        return (0, 0, 0);
    }
    let channel = |c: u16| match level {
        0 => c,
        1 => c << 1,
        _ => c + 7,
    };
    let (r, g, b) = (channel(code & 7), channel((code >> 3) & 7), channel((code >> 6) & 7));
    (
        (((r << 1) | (r >> 3)) * 255 / 31) as u8,
        (((g << 2) | (g >> 2)) * 255 / 63) as u8,
        (((b << 1) | (b >> 3)) * 255 / 31) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_offsets_follow_column_major_sprite_order() {
        // 2x2 sprite (width 1, height 1): column 1 row 0 is pattern 2.
        assert_eq!(name_offset(1 | 0 << 2 | 1 << 4 | 1 << 6), 2);
        // Horizontally flipped: column 0 shows the second pattern column.
        assert_eq!(name_offset(0x100 | 1 << 4 | 1 << 6), 2);
    }

    #[test]
    fn high_priority_plane_a_hides_low_priority_sprite_but_not_high() {
        let t = &luts()[LUT_BGOBJ];
        // Background: priority plane pixel 0x45; sprite low priority colour 0x13.
        assert_eq!(t[0x45 << 8 | 0x13], 0x85);
        // High-priority sprite wins.
        assert_eq!(t[0x45 << 8 | 0x53], 0x93);
    }
}
