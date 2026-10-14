//! Bounded, project-local WAV capture using the same execution/state path as boot.
use crate::{
    artifacts::{self, Artifact},
    context::Context,
    envelope::{Coverage, CoverageStatus, Envelope, Failure, Subject},
};
use mdcore::{audio::SAMPLE_RATE, Cartridge};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn frames() -> u64 {
    120
}
fn steps() -> u64 {
    5_000_000
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioArgs {
    pub rom: String,
    /// Resume an exact-ROM machine state, preserving oscillator and mixer phase.
    #[serde(default)]
    #[schemars(with = "String")]
    pub from_state: Option<String>,
    #[serde(default)]
    pub save_state: bool,
    /// Capture this many completed video frames after start_frame (a duration).
    #[serde(default = "frames")]
    #[schemars(range(min = 1, max = 18000))]
    pub frames: u64,
    /// Warm up before capture; relative to reset or the loaded state's frame.
    #[serde(default)]
    #[schemars(range(max = 100000))]
    pub start_frame: u64,
    /// Hard step budget includes warmup. Late clips may need a larger budget.
    #[serde(default = "steps")]
    #[schemars(range(max = 1000000000))]
    pub instructions: u64,
    /// Replay frames are relative to reset or the loaded state, including warmup.
    #[serde(default)]
    #[schemars(with = "String")]
    pub input: Option<PathBuf>,
    /// Diagnostic RAM writes at frame boundaries (merged with the replay's
    /// pokes), e.g. a sound id written to Z80 RAM. Not hardware behaviour.
    #[serde(default)]
    #[schemars(length(max = 1000))]
    pub pokes: Vec<super::input::Poke>,
    /// Log 68000 writes into Z80 RAM, the bank register, BUSREQ and RESET
    /// during the capture (after start_frame); `{}` uses the defaults.
    #[serde(default)]
    pub sound_commands: Option<super::sound_log::SoundCommandOptions>,
    /// Also export the capture's YM2612/SN76489 register writes as VGM 1.60,
    /// starting with writes that recreate the chips' state at capture start.
    #[serde(default)]
    pub vgm: bool,
    /// Console region at power-on: auto (header, like the reference), usa,
    /// japan or europe (PAL timing).
    #[serde(default)]
    pub region: super::state::RegionChoice,
    /// Also render one WAV per voice (FM1..FM6, PSG1..PSG4) by re-synthesizing
    /// the capture's register writes with the other voices silenced.
    #[serde(default)]
    pub stems: bool,
    /// Optional output DIRECTORY for an immutable digest-named .wav (and .vgm,
    /// stems); not a file name. The store ID always works.
    #[serde(default)]
    #[schemars(with = "String")]
    pub out: Option<PathBuf>,
    #[serde(default)]
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}
#[derive(Serialize, JsonSchema)]
pub struct AudioData {
    pub wav_artifact: Artifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_commands: Option<super::sound_log::SoundCommandData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vgm_artifact: Option<Artifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vgm: Option<super::vgm::VgmSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stems: Vec<Stem>,
    pub state_artifact: Option<Artifact>,
    pub starting_frame: u64,
    pub reached_frames: u64,
    pub elapsed_frames: u64,
    pub requested_start_frame: u64,
    pub requested_capture_frames: u64,
    pub started: bool,
    pub frame_target_reached: bool,
    pub termination: String,
    pub steps: u64,
    pub exceptions: u64,
    pub sample_rate: u64,
    pub channels: usize,
    pub bits_per_sample: usize,
    pub sample_frames: usize,
    pub first_sample_index: u64,
    pub next_sample_index: u64,
    pub audio_master_start: u64,
    pub audio_master_end: u64,
    pub console_master_end: u64,
    pub peak: u32,
    pub nonzero_samples: usize,
    pub dropped_sample_frames: u64,
    pub clipped_samples: u64,
    pub input_sha256: Option<String>,
    pub input_events_applied: usize,
    pub input_events_pending: usize,
    pub pokes_applied: usize,
    pub pokes_pending: usize,
}

/// Capture length bound: five NTSC minutes of retained stereo PCM (~58 MB).
const MAX_FRAMES: u64 = 18_000;
/// Long captures of busy games need ~30k steps per frame.
const MAX_STEPS: u64 = 1_000_000_000;
/// One isolated voice rendered from the capture's register writes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Stem {
    /// fm1..fm6 (fm6 includes the DAC) or psg1..psg4 (psg4 is noise).
    pub voice: String,
    pub wav_artifact: Artifact,
    pub peak: u32,
    pub rms: f64,
}

/// Retained register writes for VGM export (a DAC-heavy minute is ~1M).
const VGM_MAX_WRITES: usize = 8_000_000;

pub fn run(args: AudioArgs, context: &Context) -> Result<Envelope<AudioData>, Failure> {
    if args.frames == 0
        || args.frames > MAX_FRAMES
        || args.start_frame > 100_000
        || args.instructions > MAX_STEPS
    {
        return Err(artifacts::failure(
            "limit_exceeded",
            "audio requires frames 1..18000, start_frame <=100000 and instructions <=1000000000",
        ));
    }
    let context = context.bounded(args.timeout_ms)?;
    context.check()?;
    let path = artifacts::resolve(&context.store, &args.rom)?;
    let cart =
        Cartridge::load(&path).map_err(|e| artifacts::failure("rom_unreadable", e.to_string()))?;
    let subject = Subject {
        path: path.display().to_string(),
        sha256: cart.sha256.clone(),
        bytes: cart.data.len(),
        format: "mega-drive-rom".into(),
        architecture: Some("m68000".into()),
    };
    let replay = super::input::Replay::load_with_pokes(args.input.as_deref(), &args.pokes)?;
    if let Some(options) = &args.sound_commands {
        super::sound_log::validate(options)?;
    }
    let mut system = super::state::load(cart, args.from_state.as_deref(), args.region, &context)?;
    system.observation_limit = 512;
    let starting_frame = system.vdp.frame;
    let target = starting_frame
        .checked_add(args.start_frame + args.frames)
        .ok_or_else(|| artifacts::failure("invalid_arguments", "audio frame target overflow"))?;
    let mut next_input = 0;
    let mut next_poke = 0;
    let mut steps = 0;
    let mut exceptions = 0;
    let mut started = false;
    let mut termination = "instruction_budget";
    let mut audio_master_start = system.audio.clock();
    let mut first_sample_index = system.audio.generated_frames();
    // At most MAX_FRAMES NTSC frames (~5 min), with generous allowance for frame/reset
    // phase and instruction overshoot. Never retain unbounded PCM.
    let max_samples = (args.frames as usize + 2) * 900; // 882 per PAL frame
    while steps < args.instructions {
        if let Some(reason) = context.stop_reason() {
            termination = reason;
            break;
        }
        if system.vdp.frame >= target {
            termination = "frame_target";
            break;
        }
        while let Some(event) = replay
            .events
            .get(next_input)
            .filter(|e| e.frame <= system.vdp.frame - starting_frame)
        {
            system.set_pad(event.port, event.mask());
            next_input += 1;
        }
        let elapsed = system.vdp.frame - starting_frame;
        replay.apply_pokes(&mut system, elapsed, &mut next_poke);
        if !started && system.vdp.frame - starting_frame >= args.start_frame {
            started = true;
            audio_master_start = system.audio.clock();
            first_sample_index = system.audio.generated_frames();
            system.audio.start_capture(max_samples);
            if let Some(options) = &args.sound_commands {
                super::sound_log::install(&mut system, options);
            }
            if args.vgm || args.stems {
                system.audio.start_register_log(VGM_MAX_WRITES);
            }
        }
        let (_, exception) = system.step_instruction();
        steps += 1;
        exceptions += u64::from(exception.is_some());
        if steps % 8192 == 0 {
            context.report(steps, args.instructions);
        }
    }
    let frame_target_reached = system.vdp.frame >= target;
    if frame_target_reached {
        termination = "frame_target";
    }
    let audio_master_end = system.audio.clock();
    let next_sample_index = system.audio.generated_frames();
    if !started {
        audio_master_start = audio_master_end;
        first_sample_index = next_sample_index;
    }
    let sound_commands = super::sound_log::finish(&mut system, &context.store)?;
    let register_log = system.audio.register_log.take();
    let vgm = register_log
        .as_ref()
        .filter(|_| args.vgm)
        .map(|log| super::vgm::encode(log, audio_master_end));
    let mut stems = Vec::new();
    if let Some(log) = register_log.as_ref().filter(|_| args.stems) {
        for (index, voice) in mdcore::audio::VOICES.iter().enumerate() {
            context.check()?;
            let pcm = mdcore::audio::render_voices(log, audio_master_end, 1 << index);
            let bytes = wave(&pcm);
            let stored = artifacts::put(&context.store, &bytes)?;
            let wav_artifact = match &args.out {
                Some(out) => artifacts::named(out, &format!("audio-{voice}"), "wav", &bytes)?,
                None => stored,
            };
            let peak = pcm.iter().map(|n| i32::from(*n).unsigned_abs()).max().unwrap_or(0);
            let rms = (pcm.iter().map(|n| f64::from(*n).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
            stems.push(Stem { voice: voice.to_string(), wav_artifact, peak, rms: (rms * 10.0).round() / 10.0 });
        }
    }
    let pcm = system.audio.take_pcm();
    let peak = pcm
        .iter()
        .map(|n| i32::from(*n).unsigned_abs())
        .max()
        .unwrap_or(0);
    let nonzero_samples = pcm.iter().filter(|n| **n != 0).count();
    let encoded = wave(&pcm);
    let stored = artifacts::put(&context.store, &encoded)?;
    let wav_artifact = if let Some(out) = &args.out {
        artifacts::named(out, "audio", "wav", &encoded)?
    } else {
        stored
    };
    let (vgm_artifact, vgm) = match vgm {
        Some((bytes, summary)) => {
            let stored = artifacts::put(&context.store, &bytes)?;
            let artifact = match &args.out {
                Some(out) => artifacts::named(out, "audio", "vgm", &bytes)?,
                None => stored,
            };
            (Some(artifact), Some(summary))
        }
        None => (None, None),
    };
    let mut coverage = Coverage::complete(vec![format!(
        "{steps} system steps; stereo sample indexes {first_sample_index}..{next_sample_index}"
    )]);
    coverage.limits=vec![format!("steps <= {}; retained stereo frames <= {max_samples}; capture includes warmup in step budget",args.instructions)];
    coverage.limitations=vec![
        "NTSC YM2612 synthesis uses pinned ymfm-sys 0.2.0; PSG edge model uses 2 dB attenuation and 16-bit noise".into(),
        "44.1 kHz integer box resampling and DC removal; no analog-board filtering or hardware/Genesis Plus GX PCM equivalence claim".into(),
        "sound writes use modeled CPU bus and Z80 dispatch timing; synthesis advances only after safe scanline/VINT batches, so its reported clock can lag console execution".into(),
        "FM status visible to CPUs remains the separately modeled timer/status implementation".into(),
    ];
    if !system.cpu.branch_override.is_empty() {
        coverage
            .limitations
            .push("analyst-forced branch outcomes inherited from state; modified execution".into());
    }
    if !frame_target_reached || !started || system.audio.dropped_frames != 0 {
        coverage.status = CoverageStatus::Partial;
        coverage.limitations.push(format!(
            "run stopped with {termination}; requested audio scope did not complete"
        ));
    }
    if system.audio.dropped_frames != 0 {
        coverage.truncated.push(format!(
            "{} stereo sample frames omitted",
            system.audio.dropped_frames
        ));
    }
    Ok(Envelope::ok(
        "mdx.audio",
        Some(subject),
        coverage,
        AudioData {
            wav_artifact,
            sound_commands,
            vgm_artifact,
            vgm,
            stems,
            state_artifact: super::state::save(&system, args.save_state, &context)?,
            starting_frame,
            reached_frames: system.vdp.frame,
            elapsed_frames: system.vdp.frame - starting_frame,
            requested_start_frame: args.start_frame,
            requested_capture_frames: args.frames,
            started,
            frame_target_reached,
            termination: termination.into(),
            steps,
            exceptions,
            sample_rate: SAMPLE_RATE,
            channels: 2,
            bits_per_sample: 16,
            sample_frames: pcm.len() / 2,
            first_sample_index,
            next_sample_index,
            audio_master_start,
            audio_master_end,
            console_master_end: system.master_cycles(),
            peak,
            nonzero_samples,
            dropped_sample_frames: system.audio.dropped_frames,
            clipped_samples: system.audio.clipped_samples,
            input_sha256: replay.source_sha256,
            input_events_applied: next_input,
            input_events_pending: replay.events.len() - next_input,
            pokes_applied: next_poke,
            pokes_pending: replay.pokes.len() - next_poke,
        },
    ))
}
fn wave(pcm: &[i16]) -> Vec<u8> {
    let bytes = pcm.len() as u32 * 2;
    let mut out = Vec::with_capacity(bytes as usize + 44);
    out.extend(b"RIFF");
    out.extend((bytes + 36).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend((SAMPLE_RATE as u32).to_le_bytes());
    out.extend((SAMPLE_RATE as u32 * 4).to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(bytes.to_le_bytes());
    for sample in pcm {
        out.extend(sample.to_le_bytes());
    }
    out
}
