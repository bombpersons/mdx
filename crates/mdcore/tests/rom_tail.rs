//! Irregular-sized linear ROMs: reads past the image read FF up to the
//! power-of-two period, which then mirrors the image start. The values match
//! the unchanged-GPGX tail matrix (tools/probes/rom_tail_reads.py). A Cadash
//! image grown by two bytes, and Barney's reset prefetch, once panicked here.
use mdcore::{Cartridge, System};

fn rom(length: usize) -> Vec<u8> {
    let period = length.next_power_of_two() as u32;
    let addresses = [0, 2, length as u32 - 2, length as u32, length as u32 + 2, period - 2, period, period + 2];
    let mut code = Vec::new();
    for (i, address) in addresses.iter().enumerate() {
        // MOVE.W (address).L,D0 ; MOVE.W D0,(0xFFF000+2i).L
        code.extend([0x3039, (address >> 16) as u16, *address as u16, 0x33c0, 0x00ff, 0xf000 + i as u16 * 2]);
    }
    code.extend([0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]);
    let mut rom = vec![0x6c; length];
    rom[..8].copy_from_slice(&[0x00, 0xff, 0x80, 0x00, 0x00, 0x00, 0x02, 0x00]);
    rom[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    for (i, word) in code.iter().enumerate() {
        rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
    }
    rom[length - 2..].copy_from_slice(&[0x12, 0x34]);
    rom
}

#[test]
fn unbacked_tail_reads_ff_and_the_period_mirrors_the_image_start() {
    for length in [0x10002, 0x30002, 0x80002, 0x100004, 0x300000] {
        let mut system = System::new(Cartridge::from_bytes("tail.md", rom(length)).unwrap());
        for _ in 0..200 {
            assert_eq!(system.step_instruction().1, None, "length {length:#x}");
            if system.wram[0xf040..0xf042] == [0xc0, 0xde] {
                break;
            }
        }
        assert_eq!(system.wram[0xf040..0xf042], [0xc0, 0xde], "length {length:#x}");
        let words: Vec<u16> = system.wram[0xf000..0xf010]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        // A ROM of at most 2 MiB without a header declaration gets the
        // reference's default 64 KiB backup RAM at $200000 (initialized FF),
        // which hides the mirror there.
        // Beyond the 4 MiB cartridge window the read is unmapped: open bus
        // returns the next instruction word (MOVE.W D0,abs.l = 0x33C0).
        let mirror = if length.next_power_of_two() == 0x20_0000 {
            [0xffff, 0xffff]
        } else if length.next_power_of_two() == 0x40_0000 {
            [0x33c0, 0x33c0]
        } else {
            [0x00ff, 0x8000]
        };
        assert_eq!(
            words,
            [0x00ff, 0x8000, 0x1234, 0xffff, 0xffff, 0xffff, mirror[0], mirror[1]],
            "length {length:#x}"
        );
    }
}
