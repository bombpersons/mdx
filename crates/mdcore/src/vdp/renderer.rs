//! Snapshot renderer for Mega Drive mode 5. Register/attribute layouts match
//! GPGX's mode-5 renderer. Raster effects, interlace, DMA, and shadow/highlight
//! remain explicit limitations rather than silently approximated fidelity.
use super::{Framebuffer, Vdp};

#[derive(Clone, Copy, Default)]
struct Pixel {
    colour: u8,
    high: bool,
}

fn word(v: &Vdp, address: usize) -> u16 {
    let a = address & 0xFFFE;
    u16::from_be_bytes([v.vram[a], v.vram[a + 1]])
}

fn cell(v: &Vdp, attributes: u16, x: usize, y: usize) -> Pixel {
    let column = if attributes & 0x0800 != 0 {
        7 - (x & 7)
    } else {
        x & 7
    };
    let row = if attributes & 0x1000 != 0 {
        7 - (y & 7)
    } else {
        y & 7
    };
    let texel = v.tile_texel((attributes & 0x07FF) as usize, column, row);
    Pixel {
        colour: if texel == 0 {
            0
        } else {
            ((attributes >> 9) & 0x30) as u8 | texel
        },
        high: attributes & 0x8000 != 0,
    }
}

fn dimension(bits: u8) -> usize {
    match bits & 3 {
        0 => 32,
        1 => 64,
        3 => 128,
        _ => 32,
    }
}

fn plane(v: &Vdp, base: usize, x: i32, y: i32, width: usize, height: usize) -> Pixel {
    let x = x.rem_euclid((width * 8) as i32) as usize;
    let y = y.rem_euclid((height * 8) as i32) as usize;
    let attributes = word(v, base + ((y / 8) * width + x / 8) * 2);
    cell(v, attributes, x, y)
}

fn scroll(v: &Vdp, line: usize, plane: usize) -> i32 {
    let row = match v.registers[11] & 3 {
        0 => 0,
        1 => line & 7,
        2 => line & !7,
        _ => line,
    };
    let base = (v.registers[13] as usize & 0x3F) << 10;
    let value = word(v, base + row * 4 + plane * 2) & 0x07FF;
    ((value as i16) << 5 >> 5) as i32
}

fn vertical(v: &Vdp, x: usize, plane: usize) -> i32 {
    let column = if v.registers[11] & 4 != 0 { x / 16 } else { 0 };
    let offset = (column * 4 + plane * 2) % v.vsram.len();
    (u16::from_be_bytes([v.vsram[offset], v.vsram[offset + 1]]) & 0x03FF) as i32
}

fn sprites(v: &Vdp, y: usize, width: usize) -> Vec<Pixel> {
    let wide = width == 320;
    let base = ((v.registers[5] as usize) & if wide { 0x7E } else { 0x7F }) << 9;
    let maximum = if wide { 80 } else { 64 };
    let per_line = if wide { 20 } else { 16 };
    let mut result = vec![Pixel::default(); width];
    let mut index = 0;
    let mut visible = 0;
    for _ in 0..maximum {
        let address = base + index * 8;
        let top = (word(v, address) & 0x01FF) as i32 - 128;
        let size_link = word(v, address + 2);
        let attributes = word(v, address + 4);
        let left = (word(v, address + 6) & 0x01FF) as i32 - 128;
        let columns = ((size_link >> 10) & 3) as usize + 1;
        let rows = ((size_link >> 8) & 3) as usize + 1;
        let dy = y as i32 - top;
        if dy >= 0 && dy < (rows * 8) as i32 {
            visible += 1;
            if visible > per_line {
                break;
            }
            for dx in 0..columns * 8 {
                let x = left + dx as i32;
                if x < 0 || x >= width as i32 || result[x as usize].colour != 0 {
                    continue;
                }
                let tile_x = if attributes & 0x0800 != 0 {
                    columns - 1 - dx / 8
                } else {
                    dx / 8
                };
                let tile_y = if attributes & 0x1000 != 0 {
                    rows - 1 - dy as usize / 8
                } else {
                    dy as usize / 8
                };
                let tile = ((attributes & 0x07FF) as usize + tile_x * rows + tile_y) & 0x07FF;
                result[x as usize] = cell(v, (attributes & !0x07FF) | tile as u16, dx, dy as usize);
            }
        }
        index = (size_link & 0x7F) as usize;
        if index == 0 || index >= maximum {
            break;
        }
    }
    result
}

pub(super) fn render(v: &Vdp) -> (Framebuffer, Vec<String>) {
    let width = if v.registers[12] & 1 != 0 { 320 } else { 256 };
    let height = if v.registers[1] & 8 != 0 { 240 } else { 224 };
    let mut frame = Framebuffer::new(width, height);
    let mut limitations = Vec::new();
    if v.registers[12] & 6 != 0 {
        limitations.push("interlaced display is not modelled".into());
    }
    if v.registers[12] & 8 != 0 {
        limitations.push("shadow/highlight is not modelled".into());
    }
    if v.dma_requests != 0 {
        limitations.push("VDP DMA transfer timing is approximate".into());
    }
    if v.dma_unsupported {
        limitations.push("unsupported VDP DMA destination/mode".into());
    }
    if v.registers[16] & 3 == 2 || (v.registers[16] >> 4) & 3 == 2 {
        limitations.push("invalid plane-size encoding is not modelled".into());
    }
    let plane_width = dimension(v.registers[16]);
    let plane_height = dimension(v.registers[16] >> 4);
    let a_base = ((v.registers[2] & 0x38) as usize) << 10;
    let b_base = ((v.registers[4] & 7) as usize) << 13;
    let window_base = ((v.registers[3] as usize) & if width == 320 { 0x3C } else { 0x3E }) << 10;
    for y in 0..height {
        let sprite = sprites(v, y, width);
        for x in 0..width {
            let horizontal = if v.registers[17] & 0x80 != 0 {
                x >= (v.registers[17] as usize & 0x1F) * 16
            } else {
                x < (v.registers[17] as usize & 0x1F) * 16
            };
            let vertical_window = if v.registers[18] & 0x80 != 0 {
                y >= (v.registers[18] as usize & 0x1F) * 8
            } else {
                y < (v.registers[18] as usize & 0x1F) * 8
            };
            let a = if horizontal || vertical_window {
                plane(
                    v,
                    window_base,
                    x as i32,
                    y as i32,
                    if width == 320 { 64 } else { 32 },
                    32,
                )
            } else {
                plane(
                    v,
                    a_base,
                    x as i32 - scroll(v, y, 0),
                    y as i32 + vertical(v, x, 0),
                    plane_width,
                    plane_height,
                )
            };
            let b = plane(
                v,
                b_base,
                x as i32 - scroll(v, y, 1),
                y as i32 + vertical(v, x, 1),
                plane_width,
                plane_height,
            );
            let mut colour = v.registers[7] & 0x3F;
            if v.display_enabled() {
                for high in [false, true] {
                    for pixel in [b, a, sprite[x]] {
                        if pixel.colour != 0 && pixel.high == high {
                            colour = pixel.colour;
                        }
                    }
                }
            }
            frame.set(x, y, v.palette(colour as usize));
        }
    }
    if !v.display_enabled() {
        limitations.push("display disabled: backdrop only".into());
    }
    (frame, limitations)
}
