//! `mdx` -- an agent-first reverse-engineering toolkit for Mega Drive cartridges.
//!
//! Every command writes exactly one JSON envelope to stdout. Humans can ask for
//! a readable rendering later; the machine-readable form is the contract.

use mdx::{commands, context::Context, envelope, operations};

use clap::{Parser, Subcommand};
use envelope::{emit, Envelope, Failure};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "mdx",
    version,
    about = "Agent-first reverse-engineering toolkit for Mega Drive cartridges",
    long_about = "Every command emits one JSON envelope on stdout with an explicit \
                  coverage block. Nothing is truncated silently. `mdx schema <command>` \
                  prints the shape of a command's output."
)]
struct Cli {
    /// Base directory for relative inputs, outputs and artifact store.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[arg(long, global = true, default_value = ".mdx/artifacts")]
    store: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum RamCommand {
    /// Run the cartridge, then report what its work RAM holds.
    Dump {
        /// Path to a Mega Drive ROM image.
        #[arg(long, value_name = "PATH")]
        rom: PathBuf,
        /// How many instructions to execute before reading RAM.
        #[arg(long, default_value_t = 200_000)]
        instructions: u64,
        /// Stop just before the first instruction executed at this address.
        #[arg(long, value_name = "ADDR")]
        stop_at: Option<String>,
        /// Report every address whose longword equals this value.
        #[arg(long, value_name = "VALUE")]
        find: Option<String>,
        /// Hex-dump this much work RAM from this address, as ADDR:LEN.
        #[arg(long, value_name = "ADDR:LEN")]
        range: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Serve the typed operation registry over MCP stdio.
    Mcp {
        #[arg(long, default_value_t = 2)]
        jobs: usize,
    },
    /// Batch typed operations from a JSON arguments object.
    Survey {
        #[arg(long)]
        args: String,
    },
    /// Capture bounded instruction records as a JSONL artifact.
    Trace {
        #[arg(long)]
        args: String,
    },
    /// Observe exact folded WRAM bus writes as a JSONL artifact.
    Watch {
        #[arg(long)]
        args: String,
    },
    #[command(subcommand)]
    Artifact(ArtifactCommand),
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Invoke any typed operation with JSON arguments (identical to MCP).
    Call {
        operation: String,
        #[arg(long, default_value = "{}")]
        args: String,
    },
    /// Export every mode-5 tile, palette, name table and sprite from a verified snapshot.
    Graphics {
        #[arg(
            long,
            value_name = "PATH",
            required_unless_present = "from",
            conflicts_with = "from"
        )]
        snapshot: Option<String>,
        #[arg(long, value_name = "ARTIFACT_ID")]
        from: Option<String>,
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Render a digest-verified boot/VDP snapshot without executing the CPU.
    Render {
        #[arg(
            long,
            value_name = "PATH",
            required_unless_present = "from",
            conflicts_with = "from"
        )]
        snapshot: Option<String>,
        #[arg(long, value_name = "ARTIFACT_ID")]
        from: Option<String>,
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Disassemble the cartridge using mdcore's own decoder.
    Disasm {
        /// Path to a Mega Drive ROM image.
        #[arg(long, value_name = "PATH")]
        rom: PathBuf,
        /// First address to decode.
        #[arg(long, value_name = "ADDR")]
        from: String,
        /// How many instructions to emit.
        #[arg(long, default_value_t = 64)]
        count: usize,
    },
    /// Processor core checks.
    #[command(subcommand)]
    Cpu(CpuCommand),
    /// Run the cartridge and report what it actually does.
    Boot {
        /// JSON controller state events indexed by completed frames (zero-based).
        #[arg(long, value_name = "PATH")]
        input: Option<PathBuf>,
        /// Path to a Mega Drive ROM image.
        #[arg(long, value_name = "PATH")]
        rom: PathBuf,
        /// How many instructions to execute.
        #[arg(long, default_value_t = 200_000)]
        instructions: u64,
        /// Stop when this many emulated frames have elapsed (instruction budget
        /// remains a hard cap). Enables frame-aligned oracle comparisons.
        #[arg(long)]
        frames: Option<u64>,
        /// Include data and address registers with each traced instruction.
        #[arg(long)]
        trace_registers: bool,
        /// How many executed addresses to include in the trace.
        #[arg(long, default_value_t = 24)]
        trace: usize,
        /// Force a conditional branch, as ADDR=take|skip. Repeatable. Use to
        /// walk a cartridge down a path its condition codes would not choose.
        #[arg(long, value_name = "ADDR=take|skip")]
        force_branch: Vec<String>,
        /// Stop just before the first instruction executed at this address.
        /// Repeatable. The long way to look at a specific point in a run.
        #[arg(long, value_name = "ADDR")]
        stop_at: Vec<String>,
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Work-RAM inspection.
    #[command(subcommand)]
    Ram(RamCommand),
    /// Decode the cartridge header and exception vector table.
    Header {
        /// Path to a Mega Drive ROM image (.md / .gen / .bin).
        #[arg(long, value_name = "PATH")]
        rom: PathBuf,
    },
    /// Discover typed input/output JSON Schemas without running analysis.
    Schema {
        /// Command to describe, e.g. `header`.
        #[arg(value_name = "COMMAND")]
        command: Option<String>,
        /// Legacy compatibility flag; ignored by static schema discovery.
        #[arg(long, value_name = "PATH")]
        rom: Option<PathBuf>,
        /// Legacy compatibility flag; ignored by static schema discovery.
        #[arg(long, value_name = "PATH")]
        snapshot: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 4)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
enum ArtifactCommand {
    Import {
        #[arg(long)]
        path: String,
    },
    View {
        #[arg(long)]
        from: String,
        #[arg(long, default_value = "")]
        pointer: String,
        #[arg(long, default_value = "json")]
        format: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 16)]
        limit: usize,
    },
    Read {
        #[arg(long)]
        from: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 4096)]
        length: usize,
        #[arg(long, default_value = "hex")]
        encoding: String,
    },
}

#[derive(Subcommand, Debug)]
enum MemoryCommand {
    Read {
        #[arg(long)]
        from: String,
        #[arg(long)]
        memory: String,
        #[arg(long, default_value = "0")]
        address: String,
        #[arg(long, default_value_t = 256)]
        length: usize,
    },
    Search {
        #[arg(long)]
        from: String,
        #[arg(long)]
        memory: String,
        #[arg(long)]
        pattern: String,
        #[arg(long, default_value_t = 1)]
        alignment: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 64)]
        limit: usize,
    },
    Diff {
        #[arg(long)]
        left: String,
        #[arg(long)]
        right: String,
        #[arg(long)]
        memory: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 64)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
enum CpuCommand {
    /// Run the SingleStepTests/680x0 suite against the CPU core.
    Conformance {
        /// Root of a SingleStepTests/680x0 checkout.
        #[arg(long, value_name = "PATH")]
        suite: PathBuf,
        /// Only run instruction files whose name contains this substring.
        #[arg(long, value_name = "TEXT")]
        filter: Option<String>,
        /// How many failing cases to include in the response.
        #[arg(long, default_value_t = 20)]
        failures: usize,
        /// Legacy diagnostic: rewrite fixture memory. This invalidates canonical
        /// conformance and disables PC/prefetch comparison. Normally leave false.
        #[arg(long, action = clap::ArgAction::Set, default_value_t = false)]
        relocate: bool,
        /// Also compare total 68000 cycles with the fixture's measured length.
        #[arg(long)]
        compare_cycles: bool,
    },
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return;
        }
        Err(error) => {
            std::process::exit(usage_failure("mdx.cli", error.to_string()));
        }
    };
    if let Some(root) = cli.root {
        if let Err(error) = std::env::set_current_dir(root) {
            std::process::exit(usage_failure("mdx.cli", error.to_string()));
        }
    }
    let context = Context {
        store: cli.store,
        ..Context::default()
    };
    let code = match cli.command {
        Command::Mcp { jobs } => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("MCP runtime");
            match runtime.block_on(mdx::mcp::serve(context.clone(), jobs)) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("mdx MCP: {error}");
                    3
                }
            }
        }
        Command::Survey { args } => run_json_operation("survey", &args, &context),
        Command::Trace { args } => run_json_operation("trace", &args, &context),
        Command::Watch { args } => run_json_operation("watch", &args, &context),
        Command::Artifact(ArtifactCommand::Import { path }) => run_operation(
            "artifact.import",
            serde_json::json!({"path":path}),
            &context,
        ),
        Command::Artifact(ArtifactCommand::Read {
            from,
            offset,
            length,
            encoding,
        }) => run_operation(
            "artifact.read",
            serde_json::json!({"source":from,"offset":offset,"length":length,"encoding":encoding}),
            &context,
        ),
        Command::Artifact(ArtifactCommand::View {
            from,
            pointer,
            format,
            offset,
            limit,
        }) => run_operation(
            "artifact.view",
            serde_json::json!({"source":from,"pointer":pointer,"format":format,"offset":offset,"limit":limit}),
            &context,
        ),
        Command::Memory(MemoryCommand::Read {
            from,
            memory,
            address,
            length,
        }) => {
            let Some(address) = parse_u32(&address) else {
                std::process::exit(usage_failure("mdx.memory.read", "invalid address".into()));
            };
            run_operation(
                "memory.read",
                serde_json::json!({"snapshot":from,"memory":memory,"address":address,"length":length}),
                &context,
            )
        }
        Command::Memory(MemoryCommand::Search {
            from,
            memory,
            pattern,
            alignment,
            offset,
            limit,
        }) => run_operation(
            "memory.search",
            serde_json::json!({"snapshot":from,"memory":memory,"pattern":pattern,"alignment":alignment,"offset":offset,"limit":limit}),
            &context,
        ),
        Command::Memory(MemoryCommand::Diff {
            left,
            right,
            memory,
            offset,
            limit,
        }) => run_operation(
            "memory.diff",
            serde_json::json!({"left":left,"right":right,"memory":memory,"offset":offset,"limit":limit}),
            &context,
        ),
        Command::Call { operation, args } => match serde_json::from_str(&args) {
            Ok(value) => run_operation(&operation, value, &context),
            Err(error) => usage_failure("mdx.call", error.to_string()),
        },
        Command::Graphics {
            snapshot,
            from,
            out,
        } => run_operation(
            "graphics",
            serde_json::json!({"snapshot":snapshot.or(from).unwrap(),"out":out}),
            &context,
        ),
        Command::Render {
            snapshot,
            from,
            out,
        } => run_operation(
            "render",
            serde_json::json!({"snapshot":snapshot.or(from).unwrap(),"out":out}),
            &context,
        ),
        Command::Cpu(CpuCommand::Conformance {
            suite,
            filter,
            failures,
            relocate,
            compare_cycles,
        }) => run_conformance(
            &suite,
            filter.as_deref(),
            failures,
            relocate,
            compare_cycles,
            &context,
        ),
        Command::Boot {
            input,
            rom,
            instructions,
            frames,
            trace_registers,
            trace,
            force_branch,
            stop_at,
            out,
        } => run_boot(
            &rom,
            instructions,
            trace,
            out.as_deref(),
            trace_registers,
            &force_branch,
            &stop_at,
            frames,
            input.as_deref(),
            &context,
        ),
        Command::Ram(RamCommand::Dump {
            rom,
            instructions,
            stop_at,
            find,
            range,
        }) => run_ram_dump(&rom, instructions, stop_at, find, range, &context),
        Command::Disasm { rom, from, count } => {
            let Some(addr) = parse_u32(&from) else {
                std::process::exit(usage_failure(
                    "mdx.disasm",
                    format!("invalid address {from:?}"),
                ));
            };
            run_operation(
                "disasm",
                serde_json::json!({"rom":rom,"from":addr,"count":count}),
                &context,
            )
        }
        Command::Header { rom } => run_header(&rom, &context),
        Command::Schema {
            command,
            rom: _,
            snapshot: _,
            offset,
            limit,
        } => run_operation(
            "schema",
            serde_json::json!({"command":command,"offset":offset,"limit":limit}),
            &context,
        ),
    };
    std::process::exit(code);
}

fn run_operation(name: &str, arguments: serde_json::Value, context: &Context) -> i32 {
    emit(&operations::execute(name, arguments, context))
}

fn run_json_operation(name: &str, args: &str, context: &Context) -> i32 {
    match serde_json::from_str(args) {
        Ok(value) => run_operation(name, value, context),
        Err(error) => usage_failure(&format!("mdx.{name}"), error.to_string()),
    }
}

/// Parse `0x...` or decimal. Hex is the natural spelling for a 68000 tool but
/// decimal shows up in shell arithmetic, so accept both rather than guess.
fn parse_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        t.parse().ok()
    }
}

fn parse_addr_len(s: &str) -> Option<(u32, u32)> {
    let (a, l) = s.split_once(':')?;
    Some((parse_u32(a)?, parse_u32(l)?))
}

fn run_ram_dump(
    rom: &Path,
    instructions: u64,
    stop_at: Option<String>,
    find: Option<String>,
    range: Option<String>,
    context: &Context,
) -> i32 {
    let bad = |what: &str| Failure {
        code: "bad_argument".into(),
        message: format!("{what} is not a number: expected hex (0x1234) or decimal"),
        remediation: Some("use 0x-prefixed hex for 68000 addresses and values".into()),
    };
    let find = match find.as_deref() {
        Some(s) => match parse_u32(s) {
            Some(v) => Some(v),
            None => return emit_failed(commands::ram::TOOL, bad("--find")),
        },
        None => None,
    };
    let stop_at = match stop_at.as_deref() {
        Some(s) => match parse_u32(s) {
            Some(v) => Some(v),
            None => return emit_failed(commands::ram::TOOL, bad("--stop-at")),
        },
        None => None,
    };
    let range = match range.as_deref() {
        Some(s) => match parse_addr_len(s) {
            Some(v) => Some(v),
            None => return emit_failed(commands::ram::TOOL, bad("--range")),
        },
        None => None,
    };
    run_operation(
        "ram.dump",
        serde_json::json!({"rom":rom,"instructions":instructions,"stop_at":stop_at,"find":find,"range":range}),
        context,
    )
}

fn run_conformance(
    suite: &Path,
    filter: Option<&str>,
    failures: usize,
    relocate: bool,
    compare_cycles: bool,
    context: &Context,
) -> i32 {
    run_operation(
        "cpu.conformance",
        serde_json::json!({"suite":suite,"filter":filter,"failures":failures,"relocate":relocate,"compare_cycles":compare_cycles}),
        context,
    )
}

fn run_boot(
    rom: &Path,
    instructions: u64,
    trace: usize,
    out: Option<&Path>,
    trace_registers: bool,
    force_branches: &[String],
    stop_at: &[String],
    frames: Option<u64>,
    input: Option<&Path>,
    context: &Context,
) -> i32 {
    let stops: Vec<u32> = match stop_at
        .iter()
        .map(|a| {
            u32::from_str_radix(a.trim_start_matches("0x"), 16)
                .map_err(|_| ("usage", format!("--stop-at {a:?} is not hexadecimal")))
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(failure) => return usage_failure(commands::boot::TOOL, failure.1),
    };
    run_operation(
        "boot",
        serde_json::json!({"rom":rom,"instructions":instructions,"trace":trace,"out":out,
        "trace_registers":trace_registers,"force_branch":force_branches,"stop_at":stops,"frames":frames,"input":input}),
        context,
    )
}

fn run_header(rom: &Path, context: &Context) -> i32 {
    run_operation("header", serde_json::json!({"rom":rom}), context)
}

/// A usage failure cannot be produced by running anything, so it is synthesised.
fn usage_failure(tool: &str, message: String) -> i32 {
    let mut envelope: Envelope<serde_json::Value> = Envelope::failure(
        tool,
        Failure {
            code: "usage".into(),
            message,
            remediation: None,
        },
    );
    envelope.exit = envelope::Exit::Usage;
    let code = envelope.exit.code();
    emit(&envelope);
    code
}

fn emit_failed(tool: &str, failure: Failure) -> i32 {
    let envelope: Envelope<serde_json::Value> = Envelope::failure(tool, failure);
    emit(&envelope);
    envelope.exit.code()
}
