//! Register-log re-synthesis: with every voice kept it reproduces a power-on
//! capture exactly; isolated voices sum back to the mix.
use mdcore::audio::{render_voices, VOICES};
use mdcore::{Cartridge, System};

/// 68000 program: take the Z80 bus and release its reset, program an FM voice on channel 1 with a
/// key-on, start a PSG tone on channel 1, then idle.
fn rom() -> Vec<u8> {
    // BUSREQ, then release Z80 reset: the bus is not granted while reset holds.
    let mut code: Vec<u16> = vec![
        0x33FC, 0x0100, 0x00A1, 0x1100, 0x33FC, 0x0100, 0x00A1, 0x1200, 0x41F9, 0x00A0, 0x4000,
    ];
    let fm: &[(u16, u16)] = &[
        (0x22, 0x00), (0x27, 0x00), (0x2B, 0x00), (0x30, 0x71), (0x34, 0x0D), (0x38, 0x33),
        (0x3C, 0x01), (0x40, 0x23), (0x44, 0x2D), (0x48, 0x26), (0x4C, 0x00), (0x50, 0x5F),
        (0x54, 0x99), (0x58, 0x5F), (0x5C, 0x94), (0x60, 0x05), (0x64, 0x05), (0x68, 0x05),
        (0x6C, 0x07), (0x70, 0x02), (0x74, 0x02), (0x78, 0x02), (0x7C, 0x02), (0x80, 0x11),
        (0x84, 0x11), (0x88, 0x11), (0x8C, 0xA6), (0xB0, 0x32), (0xB4, 0xC0), (0xA4, 0x22),
        (0xA0, 0x69), (0x28, 0xF0),
    ];
    for (register, value) in fm {
        // move.b #register,(a0) ; move.b #value,1(a0)
        code.extend([0x10BC, *register, 0x117C, *value, 0x0001]);
    }
    for psg in [0x8E, 0x0F, 0x92] {
        code.extend([0x13FC, psg, 0x00C0, 0x0011]); // move.b #psg,$C00011
    }
    code.push(0x60FE);
    let mut rom = vec![0u8; 0x1000];
    rom[..4].copy_from_slice(&0x00FF_8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    rom[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    for (i, word) in code.iter().enumerate() {
        rom[0x200 + i * 2..0x202 + i * 2].copy_from_slice(&word.to_be_bytes());
    }
    rom
}

#[test]
fn resynthesis_matches_capture_and_isolated_voices_sum_to_the_mix() {
    let mut system = System::new(Cartridge::from_bytes("stems.md", rom()).unwrap());
    system.audio.start_capture(20_000);
    system.audio.start_register_log(1_000_000);
    while system.vdp.frame < 12 {
        system.step_instruction();
    }
    let end = system.audio.clock();
    let captured = system.audio.take_pcm();
    let log = system.audio.register_log.take().unwrap();
    assert_eq!(log.writes.len(), 36, "power-on FM reset, 32 FM register writes and 3 PSG bytes");
    assert!(captured.iter().any(|s| *s != 0));

    let all = render_voices(&log, end, 0x3FF);
    assert_eq!(all, captured, "full re-synthesis must equal the capture");

    let stems: Vec<Vec<i16>> = (0..VOICES.len()).map(|v| render_voices(&log, end, 1 << v)).collect();
    // This capture starts at power-on, so the full mix carries the output
    // filter's settling transient (it decays by 0.995 per sample) while stems
    // start with the filter at rest; compare after it.
    const SETTLED: usize = 2 * 2000;
    for (index, stem) in stems.iter().enumerate() {
        let audible = stem[SETTLED..].iter().any(|s| *s != 0);
        assert_eq!(audible, index == 0 || index == 6, "{} audible={audible}", VOICES[index]);
    }
    let worst = all
        .iter()
        .zip(stems[0].iter().zip(&stems[6]))
        .skip(SETTLED)
        .map(|(mix, (fm, psg))| (i32::from(*mix) - i32::from(*fm) - i32::from(*psg)).abs())
        .max()
        .unwrap();
    // The YM2612 DAC's per-channel offset depends on each channel's output
    // sign (ymfm models this "ladder" distortion), so isolated voices sum to
    // the mix only within that nonlinearity: under 1% of this mix's peak.
    let peak = all.iter().map(|s| i32::from(*s).abs()).max().unwrap();
    assert!(worst * 100 <= peak, "stems differ from the mix by up to {worst} (peak {peak})");
}
