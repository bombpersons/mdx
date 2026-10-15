//! `mdcore` -- the Mega Drive emulation and analysis engine.
//!
//! Deliberately free of Bevy and of any windowing: this crate is a library so
//! that both the `mdx` command-line tool and the eventual game can depend on
//! it. Everything it exposes is deterministic and side-effect free apart from
//! the artefact files an analysis pass is asked to write.

pub mod cart_hw;
pub mod cart_db;
pub mod svp;
pub mod eeprom;
pub mod cartridge;
pub mod audio;
pub mod cpu;
pub mod fm;
pub mod input;
pub mod memmap;
pub mod observe;
pub mod psg;
pub mod system;
pub mod vdp;
pub mod z80;

pub use cartridge::{Cartridge, SegaHeader};
pub use cpu::m68000::{Exception, M68000};
pub use cpu::{Bus, Size, SparseBus};
pub use memmap::Region;
pub use system::System;
pub use vdp::{Framebuffer, Vdp};
pub use z80::{Z80Bus, Z80};

/// Version of the engine itself, reported in every tool response.
pub const MDCORE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod test_support {
    pub fn cartridge() -> crate::Cartridge {
        let mut data = vec![0; 512 * 1024];
        data[0..4].copy_from_slice(&0xFFFF1000u32.to_be_bytes());
        data[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        data[0x100..0x200].fill(b' ');
        for (offset, text) in [
            (0x100, "SEGA MEGA DRIVE"),
            (0x110, "(C)MDX 2026.OCT"),
            (0x120, "MDX SYNTHETIC CARTRIDGE"),
            (0x150, "MDX SYNTHETIC CARTRIDGE"),
            (0x180, "GM MDXTEST -01"),
        ] {
            data[offset..offset + text.len()].copy_from_slice(text.as_bytes());
        }
        for (offset, value) in [
            (0x1a0, 0u32),
            (0x1a4, 0x7ffff),
            (0x1a8, 0xff0000),
            (0x1ac, 0xffffff),
        ] {
            data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        data[0x200..0x204].copy_from_slice(&[0x4e, 0x71, 0x60, 0xfc]);
        crate::Cartridge::from_bytes("synthetic.md", data).unwrap()
    }
}
