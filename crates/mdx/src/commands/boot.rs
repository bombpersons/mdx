//! `mdx boot` -- run the cartridge and report what it actually does.
//!
//! This is the command that settles questions a static reading of the ROM
//! cannot. Where the first executed addresses land, which vectors are taken, and
//! what the display state ends up as are all answers only a trace can give.

use crate::envelope::{Coverage, Envelope, Failure, Subject, Warning};
use mdcore::cartridge::Cartridge;
use mdcore::cpu::m68000::Exception;
use mdcore::system::System;
use mdcore::vdp::Framebuffer;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const TOOL: &str = "mdx.boot";

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootData {
    pub state_artifact: Option<crate::artifacts::Artifact>,
    pub starting_frame: u64,
    pub elapsed_frames: u64,
    pub snapshot_artifact: Option<crate::artifacts::Artifact>,
    pub termination: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub observations: BTreeMap<String, ObservationCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_replay: Option<InputReplayData>,
    /// Console region in effect (version register).
    pub console_region: mdcore::cart_hw::ConsoleRegion,
    /// 68000 writes into the sound side, when `sound_commands` was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_commands: Option<super::sound_log::SoundCommandData>,
    pub requested_frames: Option<u64>,
    /// Actual completed video frames, independent of VINT service counts.
    pub reached_frames: u64,
    pub frame_target_reached: Option<bool>,
    /// Hard attempted-system-step limit, not the number actually executed.
    pub instruction_budget: u64,
    pub memory_artifacts: BTreeMap<String, MemoryArtifact>,
    pub reset_vector_raw: String,
    pub reset_vector_on_bus: String,
    pub reset_stack_pointer: String,
    pub entry_region: String,
    pub instructions: u64,
    pub exceptions: Vec<ExceptionRecord>,
    /// The address a `--stop-at` watchpoint halted on, if one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<String>,
    /// The addresses the run was asked to watch for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop_points: Vec<String>,
    /// The first addresses actually executed, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub first_addresses: Vec<AddressRecord>,
    /// A ring of the most recently executed addresses, newest last. This is
    /// how you find what led to a fault: the prefix cannot reach past a long
    /// loop, but the tail is always the instruction that did it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub last_addresses: Vec<AddressRecord>,
    /// Most frequently executed addresses, with the symbol name if one is known.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hottest_addresses: Vec<Hotspot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_registers: Option<Registers>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vdp: Option<VdpState>,
    pub display: Display,
    pub unmapped_accesses: u64,
    /// Unhandled bus accesses, including reads; missing PC means no CPU was
    /// executing. Complete counts/omissions are in observations.unmapped_accesses.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmapped_access_samples: Vec<mdcore::observe::UnmappedAccess>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ObservationCount {
    pub observed: u64,
    pub retained: u64,
    pub omitted: u64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct InputReplayData {
    source_sha256: Option<String>,
    events: Vec<super::input::InputEvent>,
    events_applied: usize,
    events_pending: usize,
    final_pressed_masks: [u8; 2],
    /// Diagnostic RAM writes (replay file plus inline), not hardware behaviour.
    pokes_applied: usize,
    pokes_pending: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct MemoryArtifact {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ExceptionRecord {
    pub count: u64,
    pub kind: String,
    pub pc: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct AddressRecord {
    pub pc: String,
    pub region: String,
    pub cart_offset: Option<String>,
    pub opcode: String,
    /// Observed PC after the step; for a branch or return this is the target,
    /// not the sequential instruction boundary.
    pub next_pc: String,
    /// Data and address registers before the step, when `--trace-registers`
    /// is set. Enough to see a register-destination instruction go wrong
    /// without printing the whole file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registers: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Hotspot {
    pub pc: String,
    pub count: u64,
    pub region: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Registers {
    pub d: [String; 8],
    pub a: [String; 8],
    pub sr: String,
}

impl Registers {
    /// Snapshot the programmer's model. Shared with `mdx ram dump` so both
    /// report the same shape.
    pub fn of(system: &mdcore::system::System) -> Self {
        let d = std::array::from_fn(|i| format!("0x{:08X}", system.cpu.d[i]));
        let a = std::array::from_fn(|i| format!("0x{:08X}", system.cpu.a[i]));
        Registers {
            d,
            a,
            sr: format!("0x{:04X}", system.cpu.sr.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct VdpDataCounts {
    pub vram: u64,
    pub cram: u64,
    pub vsram: u64,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct VdpDataWrite {
    pub target: String,
    pub address: String,
    pub value: String,
    pub pc: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct VdpState {
    pub vram_bytes_used_nonzero: usize,
    pub cram_entries_nonzero: usize,
    pub registers: Vec<u8>,
    pub display_enabled: bool,
    pub line: u32,
    pub frame: u64,
    pub code: u8,
    pub address_latch: u32,
    pub vram_head: Vec<u8>,
    pub cram_head: Vec<u8>,
    pub z80_uploads: usize,
    /// The first 512 bytes of Z80 RAM. Sound drivers put their reset stub and
    /// the entry point there, so this window is where the code actually being
    /// executed can be read.
    pub z80_head: Vec<u8>,
    /// Sound CPU state. Reported because a driver that uploads and then does
    /// nothing is a completely different bug from one that never uploaded, and
    /// the two look identical from the 68000 side.
    pub z80_pc: u32,
    pub z80_cycles: u64,
    pub z80_in_reset: bool,
    pub z80_halted: bool,
    pub z80_rom_bank: u32,
    pub z80_ram_nonzero: usize,
    pub fm_writes: usize,
    /// Per-register FM write counts, addresses latched on port A.
    pub fm_reg_writes: Vec<u32>,
    pub fm_port_writes: Vec<u32>,
    pub z80_sound_writes: Vec<(u16, u8, u16)>,
    pub z80_port_writes: Vec<(u16, u8, u16)>,
    pub z80_port_counts: Vec<u32>,
    pub vdp_reg_writes: Vec<(u8, u8, u32)>,
    /// Data-port writes per destination: VRAM, CRAM, VSRAM.
    pub vdp_data_counts: VdpDataCounts,
    /// The first data-port writes as (target, address, value, pc).
    pub vdp_data_writes: Vec<VdpDataWrite>,
    /// The inputs to the interrupt gate at end of run, so "raised but not
    /// taken" can be split into a missing request, a mask that blocks it, or a
    /// CPU that never had interrupts enabled.
    pub irq_vint_pending: bool,
    pub irq_level: Option<u8>,
    pub irq_mask: u8,
    pub irq_enabled: bool,
    pub irq_supervisor: bool,
    pub vint_requests: u64,
    pub vint_taken: u64,
    pub dma_requests: u64,
    pub dma_bytes: u64,
    pub dma_bus_stall_master_cycles: u64,
    /// CPU wait from full VDP write FIFO, separate from external DMA waits.
    pub fifo_stall_master_cycles: u64,
    pub io_trace: Vec<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Display {
    pub screenshot_artifact: crate::artifacts::Artifact,
    pub width: usize,
    pub height: usize,
    pub screenshot_path: String,
    pub screenshot_sha256: String,
    /// SHA-256 of the decoded RGB pixels; stable across PNG encoder changes.
    pub screenshot_pixels_sha256: String,
    pub distinct_colours: usize,
    pub renderer_limitations: Vec<String>,
}

pub fn run(
    rom: &Path,
    instructions: u64,
    trace: usize,
    out_dir: Option<&Path>,
    trace_registers: bool,
    force_branches: &[String],
    stop_at: Option<Vec<u32>>,
    frames: Option<u64>,
    input: Option<&Path>,
) -> Result<(Envelope<BootData>, i32), Failure> {
    run_controlled(
        rom,
        instructions,
        trace,
        out_dir,
        trace_registers,
        force_branches,
        stop_at,
        frames,
        input,
        &crate::context::Context::default(),
    )
}

pub fn run_controlled(
    rom: &Path,
    instructions: u64,
    trace: usize,
    out_dir: Option<&Path>,
    trace_registers: bool,
    force_branches: &[String],
    stop_at: Option<Vec<u32>>,
    frames: Option<u64>,
    input: Option<&Path>,
    context: &crate::context::Context,
) -> Result<(Envelope<BootData>, i32), Failure> {
    run_resumable(
        rom,
        instructions,
        trace,
        out_dir,
        trace_registers,
        force_branches,
        stop_at,
        frames,
        input,
        &[],
        None,
        None,
        super::state::RegionChoice::Auto,
        None,
        false,
        context,
    )
}

/// Load backup RAM contents (up to 64 KiB from offset 0, e.g. a GPGX .srm).
pub fn load_sram(system: &mut mdcore::System, path: &Path) -> Result<(), Failure> {
    if !system.cart_hw.has_sram() {
        return Err(crate::artifacts::failure(
            "invalid_arguments",
            "this cartridge has no backup RAM (no header declaration, ROM over 2 MiB)",
        ));
    }
    let bytes = crate::artifacts::read_bounded(path, 0x10000)?;
    system.cart_hw.sram[..bytes.len()].copy_from_slice(&bytes);
    Ok(())
}

pub fn run_resumable(
    rom: &Path,
    instructions: u64,
    trace: usize,
    out_dir: Option<&Path>,
    trace_registers: bool,
    force_branches: &[String],
    stop_at: Option<Vec<u32>>,
    frames: Option<u64>,
    input: Option<&Path>,
    pokes: &[super::input::Poke],
    sound_commands: Option<&super::sound_log::SoundCommandOptions>,
    sram: Option<&Path>,
    region: super::state::RegionChoice,
    from_state: Option<&str>,
    save_state: bool,
    context: &crate::context::Context,
) -> Result<(Envelope<BootData>, i32), Failure> {
    if instructions > 100_000_000 || trace > 4096 || frames.is_some_and(|v| v > 100_000) {
        return Err(Failure {
            code: "limit_exceeded".into(),
            message: "boot allows at most 100M steps, 4096 inline trace records and 100000 frames"
                .into(),
            remediation: None,
        });
    }
    context.check()?;
    if stop_at.as_ref().is_some_and(|stops| {
        stops.len() > 64
            || stops
                .iter()
                .any(|address| address & 1 != 0 || *address > 0xFF_FFFF)
    }) {
        return Err(crate::artifacts::failure(
            "invalid_arguments",
            "stop points must be at most 64 even 24-bit addresses",
        ));
    }
    let replay = super::input::Replay::load_with_pokes(input, pokes)?;
    let mut next_input = 0usize;
    let mut next_poke = 0usize;
    let cart = Cartridge::load(rom).map_err(|e| Failure {
        code: "rom_unreadable".into(),
        message: e.to_string(),
        remediation: None,
    })?;
    let sha = cart.sha256.clone();
    let rom_len = cart.data.len();
    let mut system = super::state::load(cart, from_state, region, context)?;
    if let Some(options) = sound_commands {
        super::sound_log::validate(options)?;
        super::sound_log::install(&mut system, options);
    }
    if let Some(path) = sram {
        load_sram(&mut system, path)?;
    }
    let starting_frame = system.vdp.frame;
    let frame_target = frames
        .map(|f| {
            starting_frame
                .checked_add(f)
                .ok_or_else(|| crate::artifacts::failure("limit_exceeded", "frame target overflow"))
        })
        .transpose()?;
    system.observation_limit = 512;
    // `--force-branch 0xADDR=take|skip` walks a cartridge down a path its own
    // condition codes would not choose, which is how the alternate side of a
    // self-test or a feature gate gets analysed. It is a diagnostic: anything
    // derived under an override is not faithful hardware behaviour.
    for spec in force_branches {
        let Some((addr_text, take)) = spec.split_once('=') else {
            return Err(Failure {
                code: "usage".into(),
                message: format!("--force-branch expects ADDR=take|skip, got {spec:?}"),
                remediation: Some("for example --force-branch 0x7FA4C=skip".into()),
            });
        };
        let addr =
            u32::from_str_radix(addr_text.trim_start_matches("0x"), 16).map_err(|_| Failure {
                code: "usage".into(),
                message: format!("--force-branch address {addr_text:?} is not hexadecimal"),
                remediation: Some("addresses are hexadecimal, for example 0x7FA4C".into()),
            })?;
        if !matches!(take.trim(), "take" | "skip") || addr & 1 != 0 || addr > 0xFF_FFFF {
            return Err(Failure {
                code: "usage".into(),
                message: "branch override requires an even 24-bit address and take or skip".into(),
                remediation: None,
            });
        }
        system.cpu.override_branch(addr, take.trim() == "take");
    }
    let subject = Subject {
        path: system.cart.path.display().to_string(),
        sha256: sha.clone(),
        bytes: rom_len,
        format: "mega-drive-rom".into(),
        architecture: Some("m68000".into()),
    };

    let (sp, entry) = system.reset_vectors();
    let entry_region = mdcore::memmap::region(entry);

    let mut first_addresses: Vec<AddressRecord> = Vec::new();
    let mut stopped_at: Option<u32> = None;
    // Ring of recent distinct PCs; 64 is enough to span a call chain back
    // from a fault without becoming a second full trace.
    let mut tail: std::collections::VecDeque<(u32, u32, Option<String>)> =
        std::collections::VecDeque::new();
    let mut histogram: BTreeMap<u32, u64> = BTreeMap::new();
    let mut exceptions: BTreeMap<String, (u64, String, Option<String>)> = BTreeMap::new();
    let mut executed = 0u64;
    let mut pending_slot: Option<usize> = None;
    let mut termination = "instruction_budget";
    let mut tail_observed = 0u64;

    for i in 0..instructions {
        if let Some(reason) = context.stop_reason() {
            termination = reason;
            break;
        }
        if frame_target.is_some_and(|target| system.vdp.frame >= target) {
            termination = "frame_target";
            break;
        }
        while let Some(event) = replay
            .events
            .get(next_input)
            .filter(|event| event.frame <= system.vdp.frame - starting_frame)
        {
            system.set_pad(event.port, event.mask());
            next_input += 1;
        }
        let elapsed = system.vdp.frame - starting_frame;
        replay.apply_pokes(&mut system, elapsed, &mut next_poke);
        // The pc at the top of the iteration is where the previous instruction
        // decided to go, so its distance from the one before it is the length
        // that instruction actually consumed.
        if let Some(slot) = pending_slot.take() {
            let after = system.cpu.pc;
            first_addresses[slot].next_pc = format!("0x{after:08X}");
        }
        let pc = system.cpu.pc;
        let regs = trace_registers.then(|| {
            let f = |i: usize| format!("{:08X}", system.cpu.d[i]);
            let g = |i: usize| format!("{:08X}", system.cpu.a[i]);
            format!(
                // a7 is the stack pointer and drives every return in the
                // program, so it belongs in a register trace.
                "d0={} d1={} d2={} d3={} a0={} a1={} a7={} sr={:04X}",
                f(0),
                f(1),
                f(2),
                f(3),
                g(0),
                g(1),
                g(7),
                system.cpu.sr.0
            )
        });
        let opcode = format!("0x{:04X}", system.cpu.prefetch[0]);
        let traced = (i as usize) < trace;
        if traced {
            first_addresses.push(AddressRecord {
                pc: format!("0x{pc:08X}"),
                region: mdcore::memmap::region(pc).as_str().to_string(),
                cart_offset: cart_offset(pc, rom_len),
                opcode,
                next_pc: String::new(),
                registers: regs.clone(),
            });
            pending_slot = Some(first_addresses.len() - 1);
        }
        *histogram.entry(pc).or_insert(0) += 1;

        let reached_stop = stop_at.as_ref().is_some_and(|stops| stops.contains(&pc));
        if reached_stop {
            if traced {
                first_addresses.pop();
                pending_slot = None;
            }
            if let Some(count) = histogram.get_mut(&pc) {
                *count -= 1;
            }
            stopped_at = Some(pc);
            termination = "stop_point";
            break;
        }
        let outcome = system.step();
        // Recorded after the step, so the length is the one the decoder
        // actually committed to rather than where the PC started.
        if tail.back().map(|(p, _, _)| *p) != Some(pc) {
            tail_observed += 1;
            tail.push_back((pc, system.cpu.pc, regs.clone()));
            if tail.len() > 64 {
                tail.pop_front();
            }
        }
        match outcome {
            None => executed += 1,
            Some(e) => {
                let kind = describe(&e);
                let entry = exceptions.entry(kind.clone()).or_insert((
                    0,
                    format!("0x{pc:08X}"),
                    detail(&e),
                ));
                entry.0 += 1;
                executed += 1;
            }
        }
        if (i + 1) % 8192 == 0 {
            context.report(i + 1, instructions);
        }
    }

    if let Some(slot) = pending_slot {
        first_addresses[slot].next_pc = format!("0x{:08X}", system.cpu.pc);
    }
    // A target reached by the final budgeted step is still a successful stop.
    // Do not overwrite an explicit timeout/cancellation reason.
    if termination == "instruction_budget" {
        if frame_target.is_some_and(|target| system.vdp.frame >= target) {
            termination = "frame_target";
        } else if stop_at
            .as_ref()
            .is_some_and(|stops| stops.contains(&system.cpu.pc))
        {
            stopped_at = Some(system.cpu.pc);
            termination = "stop_point";
        }
    }

    let mut hottest: Vec<(u32, u64)> = histogram
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect();
    hottest.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let (frame, renderer_limitations) = system.vdp.frame();
    let distinct_colours = count_colours(&frame);

    let default_directory = context.store.join("captures");
    let directory = out_dir.unwrap_or(&default_directory);
    let screenshot = crate::artifacts::named(
        directory,
        "frame",
        "png",
        &encode_png(&frame)
            .map_err(|e| crate::artifacts::failure("screenshot_failed", e.to_string()))?,
    )?;
    let screenshot_path = screenshot.path.clone();

    let mut memory_artifacts = BTreeMap::new();
    let mut memories = vec![
        ("wram", system.wram.as_slice()),
        ("vram", system.vdp.vram.as_slice()),
        ("cram", system.vdp.cram.as_slice()),
        ("vsram", system.vdp.vsram.as_slice()),
        ("z80_ram", system.z80_ram.as_slice()),
    ];
    if system.cart_hw.has_sram() {
        memories.push(("sram", system.cart_hw.sram.as_slice()));
    }
    for (name, bytes) in memories {
        let artifact = crate::artifacts::named(directory, name, "bin", bytes)?;
        memory_artifacts.insert(
            name.to_string(),
            MemoryArtifact {
                id: artifact.id,
                path: artifact.path.display().to_string(),
                sha256: artifact.sha256,
                bytes: artifact.bytes,
            },
        );
    }

    let vram_used = system.vdp.vram.iter().filter(|b| **b != 0).count();
    let cram_used = (0..64)
        .filter(|i| system.vdp.cram[i * 2] != 0 || system.vdp.cram[i * 2 + 1] != 0)
        .count();

    let mut observations = BTreeMap::new();
    for (name, observed, retained) in [
        ("first_addresses", executed, first_addresses.len() as u64),
        ("last_addresses", tail_observed, tail.len() as u64),
        (
            "hottest_addresses",
            hottest.len() as u64,
            hottest.len().min(16) as u64,
        ),
        (
            "io_trace",
            system.io_write_count,
            system.io_log.len() as u64,
        ),
        (
            "vdp_register_writes",
            system.vdp_reg_write_count,
            system.vdp_reg_writes.len() as u64,
        ),
        (
            "vdp_data_writes",
            system.vdp_data_counts.iter().sum(),
            system.vdp_data_writes.len() as u64,
        ),
        (
            "z80_uploads",
            system.z80_upload_count,
            system.z80_upload.len() as u64,
        ),
        (
            "z80_port_writes",
            system.z80_port_counts.iter().map(|c| u64::from(*c)).sum(),
            system.z80_port_writes.len() as u64,
        ),
        (
            "z80_sound_writes",
            system.z80_sound_write_count,
            system.z80_sound_writes.len() as u64,
        ),
        (
            "unmapped_accesses",
            system.unmapped_accesses,
            system.unmapped_samples.len().min(64) as u64,
        ),
    ] {
        observations.insert(
            name.into(),
            ObservationCount {
                observed,
                retained,
                omitted: observed.saturating_sub(retained),
            },
        );
    }
    let sound_commands = super::sound_log::finish(&mut system, &context.store)?;
    let data = BootData {
        console_region: system.region(),
        sound_commands,
        state_artifact: super::state::save(&system, save_state, context)?,
        starting_frame,
        elapsed_frames: system.vdp.frame - starting_frame,
        snapshot_artifact: None,
        termination: termination.into(),
        observations,
        input_replay: Some(InputReplayData {
            source_sha256: replay.source_sha256,
            events_applied: next_input,
            events_pending: replay.events.len() - next_input,
            events: replay.events,
            final_pressed_masks: system.io.pad_buttons,
            pokes_applied: next_poke,
            pokes_pending: replay.pokes.len() - next_poke,
        }),
        requested_frames: frames,
        reached_frames: system.vdp.frame,
        frame_target_reached: frame_target.map(|target| system.vdp.frame >= target),
        instruction_budget: instructions,
        memory_artifacts,
        last_addresses: tail
            .iter()
            .map(|&(pc, after, ref regs)| AddressRecord {
                registers: regs.clone(),
                next_pc: format!("0x{after:08X}"),
                pc: format!("0x{pc:08X}"),
                region: mdcore::memmap::region(pc).as_str().to_string(),
                cart_offset: cart_offset(pc, rom_len),
                opcode: format!("0x{:04X}", opcode_at(&system, pc)),
            })
            .collect(),
        stopped_at: stopped_at.map(|a| format!("0x{a:08X}")),
        stop_points: stop_at
            .as_ref()
            .map(|v| v.iter().map(|a| format!("0x{a:08X}")).collect())
            .unwrap_or_default(),
        reset_vector_raw: format!("0x{:08X}", system.cart_long_public(4)),
        reset_vector_on_bus: format!("0x{entry:08X}"),
        reset_stack_pointer: format!("0x{sp:08X}"),
        entry_region: entry_region.as_str().to_string(),
        instructions: executed,
        exceptions: exceptions
            .into_iter()
            .map(|(kind, (count, pc, detail))| ExceptionRecord {
                count,
                kind,
                pc,
                detail,
            })
            .collect(),
        first_addresses,
        hottest_addresses: hottest
            .iter()
            .take(16)
            .map(|(pc, count)| Hotspot {
                pc: format!("0x{pc:08X}"),
                count: *count,
                region: mdcore::memmap::region(*pc).as_str().to_string(),
            })
            .collect(),
        final_registers: Some(Registers {
            d: std::array::from_fn(|i| format!("0x{:08X}", system.cpu.d[i])),
            a: std::array::from_fn(|i| format!("0x{:08X}", system.cpu.a[i])),
            sr: format!("0x{:04X}", system.cpu.sr.0),
        }),
        vdp: Some(VdpState {
            dma_requests: system.vdp.dma_requests,
            dma_bytes: system.vdp.dma_bytes,
            dma_bus_stall_master_cycles: system.vdp.dma_bus_stall_master_cycles,
            fifo_stall_master_cycles: system.vdp.fifo_stall_master_cycles,
            line: system.vdp.line,
            frame: system.vdp.frame,
            code: system.vdp.code,
            address_latch: system.vdp.address_latch,
            vram_head: system.vdp.vram[..16].to_vec(),
            cram_head: system.vdp.cram[..8].to_vec(),
            z80_uploads: system.z80_upload_count as usize,
            z80_head: system.z80_ram[..512].to_vec(),
            z80_pc: system.z80.pc as u32,
            z80_cycles: system.z80.cycles,
            z80_in_reset: system.io.z80_reset,
            z80_halted: system.z80.halted,
            z80_rom_bank: system.io.zbank,
            z80_ram_nonzero: system.z80_ram.iter().filter(|b| **b != 0).count(),
            fm_writes: system.fm.write_count,
            fm_reg_writes: system.fm.reg_writes.to_vec(),
            fm_port_writes: system.fm.port_writes.to_vec(),
            z80_sound_writes: system.z80_sound_writes.clone(),
            z80_port_writes: system.z80_port_writes.clone(),
            z80_port_counts: system.z80_port_counts.clone(),
            vdp_reg_writes: system.vdp_reg_writes.clone(),
            vdp_data_counts: VdpDataCounts {
                vram: system.vdp_data_counts[0],
                cram: system.vdp_data_counts[1],
                vsram: system.vdp_data_counts[2],
            },
            vdp_data_writes: system
                .vdp_data_writes
                .iter()
                .map(|(t, a, v, pc)| VdpDataWrite {
                    target: match t {
                        0 => "vram",
                        1 => "cram",
                        _ => "vsram",
                    }
                    .to_string(),
                    address: format!("0x{a:04X}"),
                    value: format!("0x{v:04X}"),
                    pc: format!("0x{pc:08X}"),
                })
                .collect(),
            irq_vint_pending: system.vdp.vint_pending,
            irq_level: system.irq_level(),
            irq_mask: system.cpu.sr.interrupt_mask(),
            irq_enabled: system.cpu.sr.accepts(6),
            irq_supervisor: system.cpu.sr.is_supervisor(),
            vint_requests: system.vdp.vint_requests,
            vint_taken: system.vdp.vint_taken,
            io_trace: system
                .io_log
                .iter()
                .map(|(a, v)| format!("{a:#010x}={v:#04x}"))
                .collect(),
            vram_bytes_used_nonzero: vram_used,
            cram_entries_nonzero: cram_used,
            registers: system.vdp.registers.to_vec(),
            display_enabled: system.vdp.display_enabled(),
        }),
        display: Display {
            width: frame.width,
            height: frame.height,
            screenshot_path: screenshot_path.display().to_string(),
            screenshot_sha256: screenshot.sha256.clone(),
            screenshot_pixels_sha256: pixels_sha256(&frame),
            screenshot_artifact: screenshot,
            distinct_colours: distinct_colours,
            renderer_limitations,
        },
        unmapped_accesses: system.unmapped_accesses,
        unmapped_access_samples: system.unmapped_samples.iter().take(64).cloned().collect(),
    };

    let mut warnings = Vec::new();
    if termination == "instruction_budget" {
        if let Some(target) = frames.filter(|target| system.vdp.frame < *target) {
            warnings.push(Warning::new("frame_target_not_reached",format!(
                "requested frame {target}, reached frame {}; the {instructions}-step budget stopped execution first. Increase instructions to reach the frame target; frame counts are not serviced VINT counts.",system.vdp.frame)));
        }
    }
    if entry_region != mdcore::Region::Cartridge {
        warnings.push(Warning::new(
            "entry_point_not_in_cartridge",
            format!(
                "the reset entry point decodes into {} rather than the cartridge; \
                 the trace below shows where execution actually begins",
                entry_region.as_str()
            ),
        ));
    }
    if !system.vdp.display_enabled() {
        warnings.push(Warning::new(
            "display_disabled",
            "the VDP display is still off when execution stopped",
        ));
    }
    if distinct_colours <= 1 {
        warnings.push(Warning::new(
            "frame_is_blank",
            "every rendered pixel is the same colour, so no picture has been produced yet",
        ));
    }

    let conditions = vec![
        format!("instructions[0..{executed}]"),
        format!("trace[0..{}]", data.first_addresses.len()),
        format!("frames[0..{}]", system.vdp.frame),
    ];
    let mut coverage = Coverage::complete(conditions);
    if matches!(termination, "cancelled" | "timeout") {
        coverage.status = crate::envelope::CoverageStatus::Partial;
        coverage
            .limitations
            .push(format!("run terminated with {termination}"));
    } else if !matches!(termination, "stop_point" | "frame_target") {
        if frame_target.is_some_and(|target| system.vdp.frame < target) {
            coverage.status = crate::envelope::CoverageStatus::Partial;
            coverage.limitations.push(
                "requested frame target was not reached within the instruction budget".into(),
            );
        }
        if stop_at.as_ref().is_some_and(|stops| !stops.is_empty()) && stopped_at.is_none() {
            coverage.status = crate::envelope::CoverageStatus::Partial;
            coverage
                .limitations
                .push("requested stop point was not reached within the instruction budget".into());
        }
    }
    coverage.limits = vec![
        format!("attempted_cpu_steps <= {instructions}"),
        format!("inline_trace_prefix <= {trace}"),
        "recent distinct-PC steps <= 64; hottest addresses <= 16; IO/upload samples <= 512".into(),
    ];
    coverage
        .limitations
        .extend(data.display.renderer_limitations.clone());
    coverage.limitations.push("CPU step counts include interrupt entry; register traces are pre-step; device logs are bounded samples with explicit observed/retained/omitted counts".into());
    if !system.cpu.branch_override.is_empty() {
        coverage
            .limitations
            .push("analyst-forced branch outcomes; this is a modified-execution diagnostic".into());
    }
    let mut envelope = Envelope::ok(TOOL, Some(subject), coverage, data);
    envelope.warnings = warnings;
    let code = envelope.exit.code();
    Ok((envelope, code))
}

/// The opcode word the CPU would fetch at `pc`, for addresses the tracer can
/// attribute to a file. Ram reads are not recoverable this way and report 0.
fn opcode_at(system: &System, pc: u32) -> u16 {
    match mdcore::memmap::region(pc) {
        mdcore::memmap::Region::Cartridge => {
            u16::from_be_bytes([system.cart.mapped_byte(pc), system.cart.mapped_byte(pc.wrapping_add(1))])
        }
        _ => 0,
    }
}

fn cart_offset(pc: u32, rom_len: usize) -> Option<String> {
    if mdcore::memmap::region(pc) == mdcore::Region::Cartridge
        && (mdcore::memmap::fold_to_cart_mirrored(pc, rom_len) as usize) < rom_len {
        Some(format!(
            "0x{:06X}",
            mdcore::memmap::fold_to_cart_mirrored(pc, rom_len)
        ))
    } else {
        None
    }
}

fn describe(e: &Exception) -> String {
    match e {
        Exception::Illegal { opcode } => format!("illegal_instruction:{opcode:#06X}"),
        Exception::DivideByZero => "divide_by_zero".to_string(),
        Exception::Vector { vector } => format!("vector_{vector}"),
        Exception::AddressError { address, .. } => format!("address_error:{address:#010X}"),
        Exception::Privileged => "privilege_violation".to_string(),
    }
}

fn detail(e: &Exception) -> Option<String> {
    match e {
        Exception::Vector { vector } => Some(format!("exception vector {vector}")),
        Exception::AddressError { address, .. } => Some(format!("address {address:#010X}")),
        _ => None,
    }
}

fn count_colours(frame: &Framebuffer) -> usize {
    let mut seen = std::collections::HashSet::new();
    for px in frame.data.chunks_exact(3) {
        seen.insert((px[0], px[1], px[2]));
    }
    seen.len()
}

/// SHA-256 of the decoded RGB pixels, independent of the PNG encoding.
pub(super) fn pixels_sha256(frame: &Framebuffer) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(&frame.data))
}

/// Encode RGB pixels as PNG. Frames with at most 256 colours (all Mega Drive
/// output) use an indexed palette; maximum compression with adaptive filters.
/// Decoded pixels are identical either way.
pub(super) fn encode_png(frame: &Framebuffer) -> std::io::Result<Vec<u8>> {
    let error = |e: png::EncodingError| std::io::Error::other(e.to_string());
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut lookup = std::collections::HashMap::new();
    let mut indices = Vec::with_capacity(frame.data.len() / 3);
    for px in frame.data.chunks_exact(3) {
        let colour = [px[0], px[1], px[2]];
        let index = *lookup.entry(colour).or_insert_with(|| {
            palette.push(colour);
            palette.len() - 1
        });
        if index > 255 {
            break;
        }
        indices.push(index as u8);
    }
    let indexed = palette.len() <= 256;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, frame.width as u32, frame.height as u32);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Best);
        encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        if indexed {
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_palette(palette.concat());
        } else {
            encoder.set_color(png::ColorType::Rgb);
        }
        let mut writer = encoder.write_header().map_err(error)?;
        writer
            .write_image_data(if indexed { &indices } else { &frame.data })
            .map_err(error)?;
    }
    Ok(bytes)
}
