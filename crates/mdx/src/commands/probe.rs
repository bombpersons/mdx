//! Bounded instruction and bus-write probes with pre-instruction register state.
use crate::{
    artifacts::{self, Artifact},
    context::Context,
    envelope::{Coverage, CoverageStatus, Envelope, Failure, Subject},
};
use mdcore::{
    observe::{WramWatch, WramWrite, Z80RamWatch, Z80RamWrite, Z80Trace, Z80TraceRecord},
    Cartridge,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn steps() -> u64 {
    200_000
}
fn records() -> usize {
    1024
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchRange {
    /// wram: any folded work-RAM alias. z80: 0x0000..0x1FFF or 0xA00000..0xA01FFF.
    pub address: u32,
    #[schemars(range(min = 1, max = 65536))]
    pub length: usize,
    /// Only writes by this PC (68000, or Z80 for Z80-side writes to z80 space).
    pub writing_pc: Option<u32>,
    /// "wram" (default) or "z80": Z80 sound RAM written by either CPU.
    #[serde(default)]
    pub space: WatchSpace,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WatchSpace {
    #[default]
    Wram,
    Z80,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProbeCpu {
    #[default]
    M68000,
    Z80,
}

/// A Z80 trace record with mdcore's disassembly of its bytes.
#[derive(Serialize, JsonSchema)]
pub struct Z80TraceEvent {
    #[serde(flatten)]
    pub record: Z80TraceRecord,
    pub text: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeArgs {
    /// Resumable machine state artifact ID or JSON path. ROM must match exactly.
    #[serde(default)]
    #[schemars(with = "String")]
    pub from_state: Option<String>,
    /// Publish a complete modeled machine state for later continuation.
    #[serde(default)]
    pub save_state: bool,
    pub rom: String,
    #[serde(default = "steps")]
    #[schemars(range(max = 100000000))]
    pub instructions: u64,
    #[schemars(range(max = 100000))]
    pub frames: Option<u64>,
    #[serde(default)]
    pub start_frame: u64,
    /// Begin recording when this PC is reached along with the other start gates.
    /// Once started, all subsequent instructions/writes are eligible; this is not a PC filter.
    pub start_pc: Option<u32>,
    /// Start after this many system-step attempts (includes IRQ/idle steps;
    /// not an oracle instruction ordinal). All supplied start gates must match.
    #[serde(default)]
    #[schemars(range(max = 100000000))]
    pub start_step: u64,
    pub stop_at: Option<u32>,
    #[serde(default = "records")]
    #[schemars(range(max = 100000))]
    pub max_records: usize,
    /// Controller replay JSON path; schema discovery includes file_schemas.controller_replay.
    pub input: Option<PathBuf>,
    /// Diagnostic RAM writes at frame boundaries (merged with the replay's pokes).
    #[serde(default)]
    #[schemars(length(max = 1000))]
    pub pokes: Vec<super::input::Poke>,
    pub watch: Option<WatchRange>,
    /// trace only: "m68000" (default) or "z80". Start gates (start_frame,
    /// start_step, start_pc) and stop_at still refer to the 68000 run.
    #[serde(default)]
    pub cpu: ProbeCpu,
    /// cpu z80 only: record Z80 instructions with PC in [start, end).
    pub z80_pc_range: Option<[u16; 2]>,
    /// Console region at power-on: auto (header, like the reference), usa,
    /// japan or europe (PAL timing).
    #[serde(default)]
    pub region: super::state::RegionChoice,
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct TraceEvent {
    pub step: u64,
    pub frame: u64,
    /// Exact console clock before dispatch, including reset phase and bus waits.
    pub master_cycles: u64,
    /// Exact elapsed master clocks for this step; CPU cycle costs round waits.
    pub elapsed_master_cycles: u64,
    pub pc: u32,
    pub next_pc: u32,
    pub d: [u32; 8],
    pub a: [u32; 8],
    pub sr: u16,
    pub prefetch: [u16; 2],
    pub cycles: u32,
    pub exception: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ProbeData {
    pub mode: String,
    pub state_artifact: Option<Artifact>,
    pub starting_frame: u64,
    pub elapsed_frames: u64,
    pub termination: String,
    pub started: bool,
    pub steps: u64,
    pub frames: u64,
    pub exceptions: u64,
    pub observed_records: u64,
    pub retained_records: usize,
    pub omitted_records: u64,
    pub records_artifact: Artifact,
    pub trace_preview: Vec<TraceEvent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub z80_trace_preview: Vec<Z80TraceEvent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub z80_write_preview: Vec<Z80RamWrite>,
    pub write_preview: Vec<WramWrite>,
    pub input_sha256: Option<String>,
    pub input_events_applied: usize,
    pub input_events_pending: usize,
    pub pokes_applied: usize,
    pub pokes_pending: usize,
}

pub fn run(
    mut args: ProbeArgs,
    mode: &str,
    context: &Context,
) -> Result<Envelope<ProbeData>, Failure> {
    if args.instructions > 100_000_000
        || args.start_step > args.instructions
        || args.max_records > 100_000
        || args.frames.is_some_and(|f| f > 100_000)
        || args.frames.is_some_and(|f| f <= args.start_frame)
    {
        return Err(artifacts::failure("limit_exceeded", "probe limits: 100M attempted steps, 100000 records/frames; frame target must follow start frame"));
    }
    let z80_watch = args.watch.as_ref().is_some_and(|w| w.space == WatchSpace::Z80);
    let z80_trace = mode == "trace" && args.cpu == ProbeCpu::Z80;
    if (args.cpu == ProbeCpu::Z80 && mode != "trace")
        || (args.z80_pc_range.is_some() && !z80_trace)
        || args.z80_pc_range.is_some_and(|[start, end]| start >= end)
    {
        return Err(artifacts::failure(
            "invalid_arguments",
            "cpu z80 applies to trace; z80_pc_range needs cpu z80 and start < end",
        ));
    }
    for address in [
        args.start_pc,
        args.stop_at,
        args.watch.as_ref().filter(|w| w.space == WatchSpace::Wram).and_then(|w| w.writing_pc),
    ]
    .into_iter()
    .flatten()
    {
        if address > 0xFF_FFFF || address & 1 != 0 {
            return Err(artifacts::failure(
                "invalid_arguments",
                "PC filters require even 24-bit addresses",
            ));
        }
    }
    if mode == "watch" && args.watch.is_none() {
        return Err(artifacts::failure(
            "invalid_arguments",
            "watch requires a folded work-RAM range",
        ));
    }
    if mode == "trace" && args.watch.is_some() {
        return Err(artifacts::failure(
            "invalid_arguments",
            "use the watch operation for a RAM-write range",
        ));
    }
    let context = context.bounded(args.timeout_ms)?;
    context.check()?;
    let path = artifacts::resolve(&context.store, &args.rom)?;
    args.rom = path.display().to_string();
    let cart =
        Cartridge::load(&path).map_err(|e| artifacts::failure("rom_unreadable", e.to_string()))?;
    let subject = Subject {
        path: args.rom.clone(),
        sha256: cart.sha256.clone(),
        bytes: cart.data.len(),
        format: "mega-drive-rom".into(),
        architecture: Some(if z80_trace || z80_watch { "z80" } else { "m68000" }.into()),
    };
    let replay = super::input::Replay::load_with_pokes(args.input.as_deref(), &args.pokes)?;
    let mut system = super::state::load(cart, args.from_state.as_deref(), args.region, &context)?;
    let starting_frame = system.vdp.frame;
    system.observation_limit = 512;
    let mut next_input = 0;
    let mut next_poke = 0;
    let mut attempted = 0;
    let mut exceptions = 0;
    let mut started = false;
    let mut observed = 0u64;
    let mut traces = Vec::new();
    let mut termination = "instruction_budget";
    let mut z80_range = match args.watch.as_ref().filter(|_| z80_watch) {
        Some(watch) => {
            let offset = match watch.address {
                a @ 0..=0x1FFF => a,
                a @ 0xA0_0000..=0xA0_1FFF => a & 0x1FFF,
                _ => {
                    return Err(artifacts::failure(
                        "invalid_arguments",
                        "z80 watch address must be 0x0000..0x1FFF or 0xA00000..0xA01FFF",
                    ))
                }
            };
            if watch.length == 0 || offset as usize + watch.length > 0x2000 {
                return Err(artifacts::failure(
                    "invalid_arguments",
                    "z80 watch range must lie inside the 8 KiB Z80 RAM",
                ));
            }
            Some(Z80RamWatch {
                offset: offset as u16,
                length: watch.length as u16,
                writing_pc: watch.writing_pc,
                max_records: args.max_records,
                observed: 0,
                records: Vec::new(),
            })
        }
        None => None,
    };
    let mut z80_tracer = z80_trace.then(|| Z80Trace {
        pc_range: args.z80_pc_range.map(|[start, end]| (start, end)),
        max_records: args.max_records,
        observed: 0,
        records: Vec::new(),
    });
    let range = args
        .watch
        .as_ref()
        .filter(|_| !z80_watch)
        .map(|range| {
            WramWatch::new(
                mdcore::memmap::fold_to_wram(range.address) as usize,
                range.length,
                args.max_records,
                range.writing_pc,
            )
            .ok_or_else(|| {
                artifacts::failure(
                    "invalid_arguments",
                    "watch range must contain 1..65536 bytes",
                )
            })
        })
        .transpose()?;
    let mut range = range;
    while attempted < args.instructions {
        if let Some(reason) = context.stop_reason() {
            termination = reason;
            break;
        }
        if args
            .frames
            .is_some_and(|f| system.vdp.frame - starting_frame >= f)
        {
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
        if args.stop_at == Some(system.cpu.pc) {
            termination = "stop_point";
            break;
        }
        if !started
            && attempted >= args.start_step
            && system.vdp.frame - starting_frame >= args.start_frame
            && args.start_pc.is_none_or(|pc| pc == system.cpu.pc)
        {
            started = true;
            system.wram_watch = range.take();
            system.z80_ram_watch = z80_range.take();
            system.z80_trace = z80_tracer.take();
        }
        let interrupt = system
            .irq_level()
            .is_some_and(|level| system.cpu.sr.accepts(level));
        let executing = !interrupt && !system.cpu.stopped;
        let m68k_trace = mode == "trace" && !z80_trace;
        let cpu = (m68k_trace && started && executing && traces.len() < args.max_records)
            .then(|| {
                (
                    system.cpu.pc,
                    system.cpu.d,
                    system.cpu.a,
                    system.cpu.sr.0,
                    system.cpu.prefetch,
                )
            });
        let frame = system.vdp.frame;
        let master_cycles = system.master_cycles();
        let (cycles, exception) = system.step_instruction();
        if exception.is_some() {
            exceptions += 1;
        }
        if m68k_trace && started && executing {
            observed += 1;
            if let Some(cpu) = cpu {
                traces.push(TraceEvent {
                    step: attempted,
                    frame,
                    master_cycles,
                    elapsed_master_cycles: system.master_cycles() - master_cycles,
                    pc: cpu.0,
                    next_pc: system.cpu.pc,
                    d: cpu.1,
                    a: cpu.2,
                    sr: cpu.3,
                    prefetch: cpu.4,
                    cycles,
                    exception: exception.map(|e| format!("{e:?}")),
                });
            }
        }
        attempted += 1;
        if attempted % 8192 == 0 {
            context.report(attempted, args.instructions);
        }
    }
    if args
        .frames
        .is_some_and(|f| system.vdp.frame - starting_frame >= f)
    {
        termination = "frame_target";
    }
    let writes = system
        .wram_watch
        .take()
        .map(|watch| {
            observed = watch.observed;
            watch.records
        })
        .unwrap_or_default();
    let z80_writes = system
        .z80_ram_watch
        .take()
        .map(|watch| {
            observed = watch.observed;
            watch.records
        })
        .unwrap_or_default();
    let z80_steps: Vec<Z80TraceEvent> = system
        .z80_trace
        .take()
        .map(|trace| {
            observed = trace.observed;
            trace.records
        })
        .unwrap_or_default()
        .into_iter()
        .map(|record| {
            let text = mdcore::z80::disasm::disassemble(&record.bytes, record.pc).text;
            Z80TraceEvent { record, text }
        })
        .collect();
    let retained = if z80_trace {
        z80_steps.len()
    } else if z80_watch {
        z80_writes.len()
    } else if mode == "trace" {
        traces.len()
    } else {
        writes.len()
    };
    let mut encoded = Vec::new();
    if z80_trace {
        for record in &z80_steps {
            serde_json::to_writer(&mut encoded, record).unwrap();
            encoded.push(b'\n');
        }
    } else if z80_watch {
        for record in &z80_writes {
            serde_json::to_writer(&mut encoded, record).unwrap();
            encoded.push(b'\n');
        }
    } else if mode == "trace" {
        for record in &traces {
            serde_json::to_writer(&mut encoded, record).unwrap();
            encoded.push(b'\n');
        }
    } else {
        for record in &writes {
            serde_json::to_writer(&mut encoded, record).unwrap();
            encoded.push(b'\n');
        }
    }
    let records_artifact = artifacts::put(&context.store, &encoded)?;
    let mut coverage = Coverage::complete(vec![format!(
        "{attempted} attempted CPU steps; {observed} selected {mode} records observed"
    )]);
    coverage.limits = vec![format!(
        "retained records <= {}; inline preview <= 16",
        args.max_records
    )];
    coverage.limitations=vec!["registers are sampled immediately before the writing/executing instruction or interrupt step, not mid-instruction".into(),
        "trace omits interrupt-entry and stopped-CPU idle steps; start_step counts system attempts, not oracle instructions; watch reports same-value 8/16/32-bit bus writes".into(),
        "mdcore execution evidence; CPU/VDP/sound timing is not generally cycle-accurate".into()];
    if !system.cpu.branch_override.is_empty() {
        coverage.limitations.push("analyst-forced branch outcomes preserved from machine state; modified-execution diagnostic".into());
    }
    if observed > retained as u64 {
        coverage
            .truncated
            .push(format!("records: retained {retained} of {observed}"));
        coverage.status = CoverageStatus::Partial;
    }
    if !started
        || matches!(termination, "cancelled" | "timeout")
        || args
            .frames
            .is_some_and(|f| system.vdp.frame - starting_frame < f)
        || (args.stop_at.is_some() && termination != "stop_point")
    {
        coverage.status = CoverageStatus::Partial;
        coverage.limitations.push(format!(
            "run terminated with {termination}; requested probe scope may not have completed"
        ));
    }
    Ok(Envelope::ok(
        &format!("mdx.{mode}"),
        Some(subject),
        coverage,
        ProbeData {
            state_artifact: super::state::save(&system, args.save_state, &context)?,
            starting_frame,
            elapsed_frames: system.vdp.frame - starting_frame,
            mode: mode.into(),
            termination: termination.into(),
            started,
            steps: attempted,
            frames: system.vdp.frame,
            exceptions,
            observed_records: observed,
            retained_records: retained,
            omitted_records: observed.saturating_sub(retained as u64),
            records_artifact,
            trace_preview: traces.into_iter().take(16).collect(),
            z80_trace_preview: z80_steps.into_iter().take(16).collect(),
            z80_write_preview: z80_writes.into_iter().take(16).collect(),
            write_preview: writes.into_iter().take(16).collect(),
            input_sha256: replay.source_sha256,
            input_events_applied: next_input,
            input_events_pending: replay.events.len() - next_input,
            pokes_applied: next_poke,
            pokes_pending: replay.pokes.len() - next_poke,
        },
    ))
}
