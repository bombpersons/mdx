//! VGM 1.60 export of an `audio` capture's YM2612/SN76489 register writes.
//! The file starts with writes recreating the chips' state at capture start,
//! then every write with sample-accurate waits at 44.1 kHz.
use mdcore::audio::{RegisterLog, RegisterWrite, MASTER_HZ, SAMPLE_RATE};
use serde::Serialize;

/// NTSC console clocks: the YM2612 runs at master/7, the PSG at master/15.
const YM2612_CLOCK: u32 = (MASTER_HZ / 7) as u32;
const SN76489_CLOCK: u32 = (MASTER_HZ / 15) as u32;

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct VgmSummary {
    pub total_samples: u64,
    pub initial_writes: usize,
    pub fm_writes: u64,
    pub psg_writes: u64,
    /// YM2612 DAC sample writes (register 0x2A), included in fm_writes.
    pub dac_writes: u64,
    /// Key-on events per FM channel 1..6 (register 0x28 with any operator on).
    pub fm_key_ons: [u64; 6],
    /// Register writes beyond the retained bound (the VGM is then truncated).
    pub omitted_writes: u64,
    /// FM resets during the capture; VGM cannot express them.
    pub fm_resets: u64,
    /// False when the capture began from a state saved before FM shadows were
    /// recorded; initial FM registers are then zero rather than the true state.
    pub fm_initial_state_known: bool,
}

fn wait(out: &mut Vec<u8>, mut samples: u64) {
    while samples > 0 {
        match samples {
            735 => {
                out.push(0x62);
                return;
            }
            882 => {
                out.push(0x63);
                return;
            }
            1..=16 => {
                out.push(0x70 + (samples - 1) as u8);
                return;
            }
            _ => {
                let chunk = samples.min(0xFFFF);
                out.push(0x61);
                out.extend((chunk as u16).to_le_bytes());
                samples -= chunk;
            }
        }
    }
}

fn command(out: &mut Vec<u8>, write: RegisterWrite) {
    match write {
        RegisterWrite::Fm { bank, register, value } => {
            out.extend([0x52 + bank, register, value]);
        }
        RegisterWrite::Psg { value } => out.extend([0x50, value]),
        RegisterWrite::FmReset => {}
    }
}

/// Encode `log` up to `end_master` (the capture's final synthesis clock).
pub fn encode(log: &RegisterLog, end_master: u64) -> (Vec<u8>, VgmSummary) {
    // The YM2612 runs at master/7 and the PSG at master/15 (NTSC or PAL).
    let master_hz = log.start_state.master_hz;
    let (ym2612_clock, sn76489_clock) = ((master_hz / 7) as u32, (master_hz / 15) as u32);
    let sample_at = |master: u64| {
        (u128::from(master.saturating_sub(log.start_master)) * u128::from(SAMPLE_RATE)
            / u128::from(master_hz)) as u64
    };
    let mut data = Vec::new();
    let mut summary = VgmSummary {
        total_samples: sample_at(end_master),
        initial_writes: log.initial.len(),
        fm_writes: 0,
        psg_writes: 0,
        dac_writes: 0,
        fm_key_ons: [0; 6],
        omitted_writes: log.omitted,
        fm_resets: log.fm_resets,
        fm_initial_state_known: log.fm_initial_known,
    };
    for write in &log.initial {
        command(&mut data, *write);
    }
    let mut position = 0;
    for (master, write) in &log.writes {
        let at = sample_at(*master);
        wait(&mut data, at - position);
        position = at;
        command(&mut data, *write);
        match *write {
            RegisterWrite::Fm { bank, register, value } => {
                summary.fm_writes += 1;
                summary.dac_writes += u64::from(bank == 0 && register == 0x2A);
                let channel = match value & 7 {
                    code @ 0..=2 => Some(usize::from(code)),
                    code @ 4..=6 => Some(usize::from(code) - 1),
                    _ => None,
                };
                if let (0, 0x28, Some(channel)) = (bank, register, channel) {
                    summary.fm_key_ons[channel] += u64::from(value & 0xF0 != 0);
                }
            }
            RegisterWrite::Psg { .. } => summary.psg_writes += 1,
            RegisterWrite::FmReset => {}
        }
    }
    wait(&mut data, summary.total_samples.saturating_sub(position));
    data.push(0x66);

    let mut file = vec![0u8; 0x40];
    file[0..4].copy_from_slice(b"Vgm ");
    let length = (0x40 + data.len()) as u32;
    file[0x04..0x08].copy_from_slice(&(length - 4).to_le_bytes());
    file[0x08..0x0C].copy_from_slice(&0x0000_0160u32.to_le_bytes());
    file[0x0C..0x10].copy_from_slice(&sn76489_clock.to_le_bytes());
    file[0x18..0x1C].copy_from_slice(&(summary.total_samples as u32).to_le_bytes());
    let rate: u32 = if master_hz == mdcore::audio::MASTER_HZ { 60 } else { 50 };
    file[0x24..0x28].copy_from_slice(&rate.to_le_bytes());
    // Sega PSG: white-noise feedback 0x0009, 16-bit shift register.
    file[0x28..0x2A].copy_from_slice(&0x0009u16.to_le_bytes());
    file[0x2A] = 16;
    file[0x2C..0x30].copy_from_slice(&ym2612_clock.to_le_bytes());
    // Data offset is relative to 0x34; the stream follows the 0x40-byte header.
    file[0x34..0x38].copy_from_slice(&0x0Cu32.to_le_bytes());
    file.extend(data);
    (file, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_waits_and_commands_follow_the_vgm_layout() {
        let log = RegisterLog {
            start_master: 1000,
            initial: vec![RegisterWrite::Psg { value: 0x9F }],
            fm_initial_known: true,
            writes: vec![
                (1000 + 894_887, RegisterWrite::Fm { bank: 0, register: 0x28, value: 0xF1 }),
                (1000 + MASTER_HZ, RegisterWrite::Fm { bank: 1, register: 0xA0, value: 0x22 }),
            ],
            max_writes: 10,
            omitted: 0,
            fm_resets: 0,
            start_state: mdcore::audio::Audio::new().save_state(),
        };
        let (file, summary) = encode(&log, 1000 + 2 * MASTER_HZ);
        assert_eq!(&file[0..4], b"Vgm ");
        assert_eq!(u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize, file.len() - 4);
        assert_eq!(u32::from_le_bytes(file[0x2C..0x30].try_into().unwrap()), 7_670_453);
        assert_eq!(u32::from_le_bytes(file[0x0C..0x10].try_into().unwrap()), 3_579_545);
        assert_eq!(summary.total_samples, 88_200);
        assert_eq!(summary.fm_key_ons, [0, 1, 0, 0, 0, 0]);
        // PSG init, one frame wait (735 samples), key-on, waits to one second, bank-1 write.
        assert_eq!(&file[0x40..0x46], &[0x50, 0x9F, 0x62, 0x52, 0x28, 0xF1]);
        assert_eq!(&file[0x46..0x49], &[0x61, 0x65, 0xA9]); // 43365 = 44100 - 735
        assert_eq!(&file[0x49..0x4C], &[0x53, 0xA0, 0x22]);
        assert_eq!(*file.last().unwrap(), 0x66);
    }
}
