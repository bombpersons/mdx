//! Bounded instruction and bus-write probes with pre-instruction register state.
use crate::{
    artifacts::{self, Artifact},
    context::Context,
    envelope::{Coverage, CoverageStatus, Envelope, Failure, Subject},
};
use mdcore::{
    observe::{WramWatch, WramWrite},
    Cartridge, System,
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
    pub address: u32,
    #[schemars(range(min = 1, max = 65536))]
    pub length: usize,
    pub writing_pc: Option<u32>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeArgs {
    pub rom: String,
    #[serde(default = "steps")]
    #[schemars(range(max = 100000000))]
    pub instructions: u64,
    #[schemars(range(max = 100000))]
    pub frames: Option<u64>,
    #[serde(default)]
    pub start_frame: u64,
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
    pub input: Option<PathBuf>,
    pub watch: Option<WatchRange>,
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct TraceEvent {
    pub step: u64,
    pub frame: u64,
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
    pub write_preview: Vec<WramWrite>,
    pub input_sha256: Option<String>,
    pub input_events_applied: usize,
    pub input_events_pending: usize,
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
    for address in [
        args.start_pc,
        args.stop_at,
        args.watch.as_ref().and_then(|w| w.writing_pc),
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
        architecture: Some("m68000".into()),
    };
    let replay = super::input::Replay::load(args.input.as_deref())?;
    let mut system = System::new(cart);
    system.observation_limit = 512;
    let mut next_input = 0;
    let mut attempted = 0;
    let mut exceptions = 0;
    let mut started = false;
    let mut observed = 0u64;
    let mut traces = Vec::new();
    let mut termination = "instruction_budget";
    let range = args
        .watch
        .as_ref()
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
        if args.frames.is_some_and(|f| system.vdp.frame >= f) {
            termination = "frame_target";
            break;
        }
        while let Some(event) = replay
            .events
            .get(next_input)
            .filter(|e| e.frame <= system.vdp.frame)
        {
            system.io.pad_buttons[event.port] = event.mask();
            next_input += 1;
        }
        if args.stop_at == Some(system.cpu.pc) {
            termination = "stop_point";
            break;
        }
        if !started
            && attempted >= args.start_step
            && system.vdp.frame >= args.start_frame
            && args.start_pc.is_none_or(|pc| pc == system.cpu.pc)
        {
            started = true;
            system.wram_watch = range.take();
        }
        let interrupt = system
            .irq_level()
            .is_some_and(|level| system.cpu.sr.accepts(level));
        let executing = !interrupt && !system.cpu.stopped;
        let cpu = (mode == "trace" && started && executing && traces.len() < args.max_records)
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
        let (cycles, exception) = system.step_instruction();
        if exception.is_some() {
            exceptions += 1;
        }
        if mode == "trace" && started && executing {
            observed += 1;
            if let Some(cpu) = cpu {
                traces.push(TraceEvent {
                    step: attempted,
                    frame,
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
    if args.frames.is_some_and(|f| system.vdp.frame >= f) {
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
    let retained = if mode == "trace" {
        traces.len()
    } else {
        writes.len()
    };
    let mut encoded = Vec::new();
    if mode == "trace" {
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
    if observed > retained as u64 {
        coverage
            .truncated
            .push(format!("records: retained {retained} of {observed}"));
        coverage.status = CoverageStatus::Partial;
    }
    if !started
        || matches!(termination, "cancelled" | "timeout")
        || args.frames.is_some_and(|f| system.vdp.frame < f)
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
            write_preview: writes.into_iter().take(16).collect(),
            input_sha256: replay.source_sha256,
            input_events_applied: next_input,
            input_events_pending: replay.events.len() - next_input,
        },
    ))
}
