//! Optional, out-of-process Ghidra analysis (ported from psx's integration).
//! A query imports and analyses its ROM automatically; analysis manifests and
//! decompile results are reusable. mdx computes the Mega Drive memory map,
//! vectors, header, cartridge bank mapping and runtime evidence; the bundled
//! HeadlessScript only applies them.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Args, Subcommand, ValueEnum};
use mdcore::{cart_hw::CartHw, Cartridge};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::artifacts::{self, digest as sha256_bytes};
use crate::context::Context;
use crate::envelope::{Coverage, CoverageStatus, Envelope, Failure, Subject, Warning};

const SCRIPT: &str = include_str!("../../ghidra/MdxGhidra.java");
const SCRIPT_NAME: &str = "MdxGhidra.java";
/// Ghidra 12 has no plain-68000 variant; `default` decodes a 68040 superset.
const LANGUAGE_68K: &str = "68000:BE:32:default";
const LANGUAGE_Z80: &str = "z80:LE:16:default";
const VALIDATED_VERSION: &str = "12.1.4";
const FORMAT: u32 = 1;
const FILE_LIMIT: u64 = 64 * 1024 * 1024;
const MAX_EXECUTED: usize = 262_144;
const MAX_FLOWS: usize = 65_536;
const LISTING_LIMIT: usize = 2048;
const WRAM: u32 = 0xFF_0000;
type Result<T> = std::result::Result<T, Failure>;

fn fail(code: &str, message: impl ToString) -> Failure {
    artifacts::failure(code, message.to_string())
}
fn io(e: impl ToString) -> Failure {
    fail("io", e)
}
fn invalid(e: impl ToString) -> Failure {
    fail("invalid_arguments", e)
}
fn unavailable(message: impl ToString) -> Failure {
    Failure {
        code: "ghidra_unavailable".into(),
        message: message.to_string(),
        remediation: Some(
            "Install Ghidra 12 (https://ghidra-sre.org) with a JDK 21, then set MDX_GHIDRA_HOME to its directory or pass ghidra_home. Other mdx operations do not need Ghidra."
                .into(),
        ),
    }
}
fn timeout() -> u64 {
    600
}
fn function_timeout() -> u32 {
    30
}
fn limit() -> usize {
    100
}
fn stamp() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
fn cancelled(ctx: &Context) -> bool {
    ctx.cancellation.is_cancelled()
}

/// Hex with optional `0x` or `$` prefix, as 68000 listings write it.
fn hex(s: &str) -> Result<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).or_else(|| t.strip_prefix('$')).unwrap_or(t);
    u32::from_str_radix(t, 16).map_err(|_| invalid(format!("invalid hex address {s:?}")))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Cpu {
    /// The 68000 program: the cartridge window plus work RAM.
    #[default]
    M68000,
    /// A Z80 sound driver: from a ROM offset or a machine state's Z80 RAM.
    Z80,
}
impl Cpu {
    fn name(self) -> &'static str {
        match self {
            Cpu::M68000 => "m68000",
            Cpu::Z80 => "z80",
        }
    }
    fn language(self) -> &'static str {
        match self {
            Cpu::M68000 => LANGUAGE_68K,
            Cpu::Z80 => LANGUAGE_Z80,
        }
    }
    /// One canonical address per byte: 68000 addresses ignore A24-A31 and work
    /// RAM repeats through 0xE00000-0xFFFFFF, so `($8000).w` = 0xFFFF8000 = 0xFF8000.
    fn fold(self, addr: u32) -> u32 {
        match self {
            Cpu::M68000 => {
                let a = addr & 0xFF_FFFF;
                if a >= 0xE0_0000 {
                    WRAM | (a & 0xFFFF)
                } else {
                    a
                }
            }
            Cpu::Z80 => addr & 0xFFFF,
        }
    }
}

/// Fields shared by every query that can import a ROM. The macro keeps them
/// flat (no serde flatten) so tool schemas reject unknown arguments.
macro_rules! with_input {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident: $t:ty,)* }) => {
        $(#[$m])*
        #[derive(Args, Debug, Clone, Deserialize, Serialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            /// Mega Drive ROM path or sha256: artifact ID. Imported and analysed
            /// automatically (about a minute the first time; then cached).
            #[arg(long)]
            #[serde(default)]
            pub rom: Option<String>,
            /// Analysis manifest path returned by an earlier ghidra call, instead of rom.
            #[arg(long, conflicts_with = "rom")]
            #[serde(default)]
            pub analysis: Option<String>,
            /// "m68000" (default) or "z80" (sound driver; give z80_rom_offset or from_state).
            #[arg(long, value_enum, default_value = "m68000")]
            #[serde(default)]
            pub cpu: Cpu,
            /// Machine state ID or path. m68000: its work RAM (for RAM-resident code)
            /// and cartridge bank mapping. z80: analyse its Z80 RAM.
            #[arg(long)]
            #[serde(default)]
            pub from_state: Option<String>,
            /// Runtime evidence: trace coverage_artifact (best: whole-run distinct PCs
            /// and transfers), trace/watch records artifacts (JSONL) or boot result
            /// JSON, as IDs or paths. Executed PCs seed code; observed indirect
            /// jump/call targets become references and switch tables.
            #[arg(long)]
            #[serde(default)]
            pub evidence: Vec<String>,
            /// Extra code ranges LO-HI in hex (end exclusive), e.g. RAM code with from_state.
            #[arg(long = "range")]
            #[serde(default)]
            pub ranges: Vec<String>,
            /// Extra hex function entry points.
            #[arg(long = "entry")]
            #[serde(default)]
            pub entries: Vec<String>,
            /// m68000 bank override SLOT=PAGE (decimal): 512 KiB window slot 1..7
            /// shows ROM page PAGE (SSF2-style mapper). Default: reset-time mapping,
            /// or the from_state mapping.
            #[arg(long = "bank")]
            #[serde(default)]
            pub banks: Vec<String>,
            /// Register context NAME=HEX over all code, e.g. A5=312 for code that keeps a
            /// base register constant (see evidence register_hints).
            #[arg(long = "register")]
            #[serde(default)]
            pub registers: Vec<String>,
            /// Hex RAM addresses ADDR or ADDR:LEN (LEN hex, default 1) that interrupt
            /// handlers or hardware change, e.g. FFE4EE:4 for a counter the VBlank
            /// handler decrements. The decompiler then keeps every read of them instead
            /// of folding a polling loop into an infinite one.
            #[arg(long = "volatile")]
            #[serde(default)]
            pub volatile: Vec<String>,
            /// z80: ROM file offset (hex) of the driver bytes to analyse.
            #[arg(long)]
            #[serde(default)]
            pub z80_rom_offset: Option<String>,
            /// z80 with z80_rom_offset: Z80 address (hex) the first byte is uploaded to; default 0.
            #[arg(long)]
            #[serde(default)]
            pub z80_origin: Option<String>,
            /// z80 with z80_rom_offset: bytes to take (default: up to the end of Z80 RAM).
            #[arg(long)]
            #[serde(default)]
            pub z80_size: Option<u32>,
            /// Name seeds: .symbols.json from ghidra_export_symbols (bytes verified) or
            /// plain "ADDR NAME [function|data [SIZE]]" lines (unverified).
            #[arg(long)]
            #[serde(default)]
            pub symbols: Vec<String>,
            /// Ghidra installation; else MDX_GHIDRA_HOME, PATH, /opt, external/ or ~/tools.
            #[arg(long)]
            #[serde(default)]
            pub ghidra_home: Option<PathBuf>,
            /// Total deadline in seconds including import, analysis and queueing (1..=3600).
            #[arg(long, default_value_t = timeout())]
            #[serde(default = "timeout")]
            #[schemars(range(min = 1, max = 3600))]
            pub timeout: u64,
            /// Build a fresh project even if a cached analysis exists.
            #[arg(long)]
            #[serde(default)]
            pub reanalyze: bool,
            $($(#[$fm])* pub $f: $t,)*
        }
        impl $name {
            fn input(&self) -> Input {
                Input {
                    rom: self.rom.clone(),
                    analysis: self.analysis.clone(),
                    cpu: self.cpu,
                    from_state: self.from_state.clone(),
                    evidence: self.evidence.clone(),
                    ranges: self.ranges.clone(),
                    entries: self.entries.clone(),
                    banks: self.banks.clone(),
                    registers: self.registers.clone(),
                    volatile: self.volatile.clone(),
                    z80_rom_offset: self.z80_rom_offset.clone(),
                    z80_origin: self.z80_origin.clone(),
                    z80_size: self.z80_size,
                    symbols: self.symbols.clone(),
                    ghidra_home: self.ghidra_home.clone(),
                    timeout: self.timeout,
                    reanalyze: self.reanalyze,
                }
            }
        }
    };
}

#[derive(Debug, Clone, Default)]
struct Input {
    rom: Option<String>,
    analysis: Option<String>,
    cpu: Cpu,
    from_state: Option<String>,
    evidence: Vec<String>,
    ranges: Vec<String>,
    entries: Vec<String>,
    banks: Vec<String>,
    registers: Vec<String>,
    volatile: Vec<String>,
    z80_rom_offset: Option<String>,
    z80_origin: Option<String>,
    z80_size: Option<u32>,
    symbols: Vec<String>,
    ghidra_home: Option<PathBuf>,
    timeout: u64,
    reanalyze: bool,
}

with_input!(
    /// Create or reuse an analysis (optional: queries analyse automatically).
    pub struct AnalyzeArgs {}
);
with_input!(
    pub struct DecompileArgs {
        /// Hex address anywhere inside the function, e.g. a writer PC from watch.
        #[arg(long)]
        pub addr: String,
        /// Per-function decompiler budget in seconds (1..=300).
        #[arg(long, default_value_t = function_timeout())]
        #[serde(default = "function_timeout")]
        #[schemars(range(min = 1, max = 300))]
        pub decompile_timeout: u32,
        /// Keep only the function, pseudocode, callers/callees and analysis
        /// identity: drops the instruction listing, mapping and provenance, so
        /// large functions stay inline instead of spilling to result_artifact.
        #[arg(long)]
        #[serde(default)]
        pub brief: bool,
    }
);
with_input!(
    pub struct FunctionsArgs {
        /// Case-insensitive substring of the name; unnamed functions are FUN_<address>.
        #[arg(long)]
        #[serde(default)]
        pub filter: Option<String>,
        #[arg(long, default_value_t = 0)]
        #[serde(default)]
        pub offset: usize,
        /// Maximum rows (1..=1000).
        #[arg(long, default_value_t = limit())]
        #[serde(default = "limit")]
        #[schemars(range(min = 1, max = 1000))]
        pub limit: usize,
    }
);
with_input!(
    pub struct XrefsArgs {
        /// Hex address: code, ROM data, work RAM (any alias) or a hardware register.
        #[arg(long)]
        pub addr: String,
        #[arg(long, default_value_t = 0)]
        #[serde(default)]
        pub offset: usize,
        /// Maximum rows in each direction (1..=1000).
        #[arg(long, default_value_t = limit())]
        #[serde(default = "limit")]
        #[schemars(range(min = 1, max = 1000))]
        pub limit: usize,
    }
);

#[derive(Args, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoctorArgs {
    /// Ghidra installation; else MDX_GHIDRA_HOME, PATH or common install locations.
    #[arg(long)]
    #[serde(default)]
    pub ghidra_home: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnnotateArgs {
    /// Analysis manifest path.
    #[arg(long)]
    pub analysis: String,
    /// JSON file {"functions":[{"addr":"1A2C","name":"UpdatePlayer","comment":"evidence..."}],
    /// "labels":[{"addr":"FFE800","name":"player_x"}]}.
    #[arg(long)]
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// The same patches inline, instead of file.
    #[arg(skip)]
    #[serde(default)]
    pub functions: Vec<FunctionPatch>,
    /// Data names inline: RAM globals, ROM tables, any mapped address.
    #[arg(skip)]
    #[serde(default)]
    pub labels: Vec<LabelPatch>,
    #[arg(long)]
    #[serde(default)]
    pub ghidra_home: Option<PathBuf>,
    #[arg(long, default_value_t = timeout())]
    #[serde(default = "timeout")]
    #[schemars(range(min = 1, max = 3600))]
    pub timeout: u64,
}

#[derive(Args, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportArgs {
    /// Analysis manifest path.
    #[arg(long)]
    pub analysis: String,
    /// Output path ending .symbols.json; default beside the analysis. Also
    /// published to the artifact store.
    #[arg(long)]
    #[serde(default)]
    pub out: Option<PathBuf>,
    #[arg(long, default_value_t = timeout())]
    #[serde(default = "timeout")]
    #[schemars(range(min = 1, max = 3600))]
    pub timeout: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    #[serde(default)]
    pub functions: Vec<FunctionPatch>,
    #[serde(default)]
    pub labels: Vec<LabelPatch>,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabelPatch {
    /// Hex address of the data (work RAM, ROM table, SRAM, ...).
    pub addr: String,
    pub name: String,
    /// Bytes the global spans (1, 2 or 4): defines byte/word/dword data there,
    /// so a long access reads `name`, not `_name`.
    pub size: Option<u32>,
    /// End-of-line comment at the address.
    pub comment: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FunctionPatch {
    /// Hex function entry.
    pub addr: String,
    pub name: Option<String>,
    pub comment: Option<String>,
}

/// `mdx ghidra <action>`: each maps onto one typed operation.
#[derive(Subcommand, Debug)]
pub enum Action {
    /// Check the optional Ghidra installation and Java runtime.
    Doctor(DoctorArgs),
    /// Create/reuse an analysis (optional: queries do this automatically).
    Analyze(AnalyzeArgs),
    /// List analysed functions, with name filtering and pagination.
    Functions(FunctionsArgs),
    /// Pseudocode, listing and callers/callees of the function containing an address.
    Decompile(DecompileArgs),
    /// References to and from an address (code or data).
    Xrefs(XrefsArgs),
    /// Apply function renames and comments as a new analysis revision.
    Annotate(AnnotateArgs),
    /// Export byte-bound function names as .symbols.json.
    ExportSymbols(ExportArgs),
}
impl Action {
    /// The operation name and its JSON arguments.
    pub fn operation(self) -> (&'static str, Value) {
        let v = |a: &dyn erased::Ser| a.value();
        match self {
            Self::Doctor(a) => ("ghidra.doctor", v(&a)),
            Self::Analyze(a) => ("ghidra.analyze", v(&a)),
            Self::Functions(a) => ("ghidra.functions", v(&a)),
            Self::Decompile(a) => ("ghidra.decompile", v(&a)),
            Self::Xrefs(a) => ("ghidra.xrefs", v(&a)),
            Self::Annotate(a) => ("ghidra.annotate", v(&a)),
            Self::ExportSymbols(a) => ("ghidra.export_symbols", v(&a)),
        }
    }
}
mod erased {
    pub trait Ser {
        fn value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn value(&self) -> serde_json::Value {
            serde_json::to_value(self).expect("serializable arguments")
        }
    }
}

// ---------------------------------------------------------------- installation

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Installation {
    home: PathBuf,
    launcher: PathBuf,
    version: String,
    build_sha256: String,
}
fn installation(explicit: Option<&Path>) -> Result<Installation> {
    fn inspect(p: &Path) -> Result<Installation> {
        let home = p.canonicalize().map_err(|e| unavailable(format!("{}: {e}", p.display())))?;
        let launcher = home.join(if cfg!(windows) { "support/analyzeHeadless.bat" } else { "support/analyzeHeadless" });
        let props = fs::read_to_string(home.join("Ghidra/application.properties"))
            .map_err(|e| unavailable(format!("{} is not a Ghidra installation: {e}", home.display())))?;
        let version = props.lines().find_map(|s| s.strip_prefix("application.version=")).unwrap_or("unknown").to_string();
        if !launcher.is_file() {
            return Err(unavailable(format!("missing {}", launcher.display())));
        }
        Ok(Installation { home, launcher, version, build_sha256: sha256_bytes(props.as_bytes()) })
    }
    if let Some(p) = explicit {
        return inspect(p);
    }
    if let Some(p) = std::env::var_os("MDX_GHIDRA_HOME") {
        return inspect(Path::new(&p));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if let Ok(p) = dir.join("analyzeHeadless").canonicalize() {
                if let Some(home) = p.parent().and_then(Path::parent) {
                    if let Ok(i) = inspect(home) {
                        return Ok(i);
                    }
                }
            }
        }
    }
    let mut roots = vec![PathBuf::from("/opt"), PathBuf::from("external")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("tools"));
    }
    let mut candidates = Vec::new();
    for root in roots {
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().to_lowercase().starts_with("ghidra") {
                    if let Ok(i) = inspect(&entry.path()) {
                        candidates.push(i);
                    }
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.home.cmp(&b.home));
    candidates.pop().ok_or_else(|| unavailable("No Ghidra installation found (MDX_GHIDRA_HOME unset; none on PATH, /opt, external/ or ~/tools)."))
}

// ------------------------------------------------------------------ files/locks

fn read_json(path: &Path) -> Result<Value> {
    if fs::metadata(path).map_err(|e| io(format!("{}: {e}", path.display())))?.len() > FILE_LIMIT {
        return Err(fail("limit_exceeded", format!("{} exceeds 64 MiB", path.display())));
    }
    serde_json::from_slice(&fs::read(path).map_err(io)?).map_err(|e| fail("invalid_artifact", format!("{}: {e}", path.display())))
}
fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", stamp()));
    fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(io)?).map_err(io)?;
    fs::rename(tmp, path).map_err(io)
}

/// One owner per analysis directory. Never remove a lock left by another
/// process; interrupted locks require explicit inspection by the user.
struct Lock(PathBuf);
const LOCK_MARKER: &str = "mdx-ghidra-lock-v1\n";
impl Lock {
    fn acquire(dir: &Path) -> Result<Self> {
        let path = dir.join("mdx.lock");
        // Publish a fully written marker atomically so a concurrent reader never
        // mistakes an empty, newly created lock for a foreign one.
        let temp = dir.join(format!("lock-{}.tmp", stamp()));
        let mut f = OpenOptions::new().write(true).create_new(true).open(&temp).map_err(io)?;
        let staged = Self(temp);
        write!(f, "{LOCK_MARKER}pid={}\n", std::process::id()).map_err(io)?;
        match fs::hard_link(&staged.0, &path) {
            Ok(()) => Ok(Self(path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(fail(
                "analysis_busy",
                format!("{} is owned by another query; inspect interrupted owners before removing locks", path.display()),
            )),
            Err(e) => Err(io(e)),
        }
    }
    fn queued(dir: &Path, deadline: Instant, ctx: &Context) -> Result<Self> {
        loop {
            if cancelled(ctx) {
                return Err(fail("cancelled", "cancelled while queued for this analysis"));
            }
            if Instant::now() >= deadline {
                return Err(fail("timeout", "deadline reached while queued for this analysis; an interrupted owner may need inspection"));
            }
            match Self::acquire(dir) {
                Ok(lock) => return Ok(lock),
                Err(e) if e.code == "analysis_busy" => {
                    // Only mdx-owned locks queue; foreign locks remain errors.
                    let mut b = String::new();
                    match File::open(dir.join("mdx.lock")) {
                        Ok(f) => {
                            use std::io::Read;
                            f.take(128).read_to_string(&mut b).map_err(io)?;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => return Err(io(e)),
                    }
                    if !b.starts_with(LOCK_MARKER) {
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(e),
            }
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Runs in its own Unix process group so timeout/cancellation kills Java and
/// its native decompiler, not just the launcher script. No shell interpolation.
fn run_worker(command: &mut Command, log: &Path, deadline: Instant, ctx: &Context) -> Result<()> {
    if cfg!(not(unix)) {
        return Err(fail("unsupported_platform", "Ghidra worker supervision currently requires Unix."));
    }
    if cancelled(ctx) {
        return Err(fail("cancelled", "Ghidra query cancelled before worker launch"));
    }
    if Instant::now() >= deadline {
        return Err(fail("timeout", "Ghidra query deadline expired before worker launch"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let output = File::create(log).map_err(io)?;
    command.stdout(output.try_clone().map_err(io)?).stderr(output).stdin(Stdio::null());
    let mut child = command.spawn().map_err(|e| fail("worker_unavailable", format!("{e}; log: {}", log.display())))?;
    loop {
        let stop = cancelled(ctx);
        if stop || Instant::now() >= deadline {
            #[cfg(unix)]
            {
                let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(fail(
                if stop { "cancelled" } else { "timeout" },
                format!("Ghidra worker stopped; interrupted analysis is not reused. Log: {}", log.display()),
            ));
        }
        match child.try_wait().map_err(io)? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return Err(fail("ghidra_failed", format!("Ghidra exited {status}; inspect {}", log.display()))),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn fingerprint(dir: &Path) -> Result<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) -> Result<()> {
        let mut entries = fs::read_dir(dir).map_err(io)?.collect::<std::result::Result<Vec<_>, _>>().map_err(io)?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let m = e.metadata().map_err(io)?;
            if m.is_dir() {
                out.push(name);
                walk(&e.path(), out)?;
            } else if !name.ends_with(".lock") && !name.ends_with(".lock~") {
                out.push(format!(
                    "{name}:{}:{}",
                    m.len(),
                    m.modified().map_err(io)?.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
                ));
            }
        }
        Ok(())
    }
    let mut rows = Vec::new();
    walk(dir, &mut rows)?;
    Ok(sha256_bytes(rows.join("\n").as_bytes()))
}
fn gui_lock(dir: &Path) -> bool {
    ["project.lock", "project.lock~"].iter().any(|name| dir.join(name).exists())
}
/// Removes a project mdx created and has superseded. Projects open in the GUI
/// or edited outside mdx (fingerprint mismatch) are left alone.
fn discard_project(dir: &Path, expected_fingerprint: Option<&str>) {
    if gui_lock(dir) || expected_fingerprint.is_some_and(|f| fingerprint(dir).ok().as_deref() != Some(f)) {
        return;
    }
    let _ = fs::remove_dir_all(dir);
}
/// Deletes a fresh project unless the import publishes it.
struct PendingProject(Option<PathBuf>);
impl PendingProject {
    fn keep(&mut self) {
        self.0 = None;
    }
}
impl Drop for PendingProject {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            discard_project(&dir, None);
        }
    }
}

/// Ghidra 12 rejects project paths with any hidden component (e.g. `.mdx`,
/// or a checkout under `.claude/`), so the database lives apart from manifests.
fn projects_root(ctx: &Context, warnings: &mut Vec<String>) -> Result<PathBuf> {
    let hidden = |p: &Path| p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.') && c.as_os_str() != "." && c.as_os_str() != "..");
    if let Some(p) = std::env::var_os("MDX_GHIDRA_PROJECTS") {
        let p = PathBuf::from(p);
        if !p.is_absolute() || hidden(&p) {
            return Err(invalid("MDX_GHIDRA_PROJECTS must be an absolute path without hidden (dot) components"));
        }
        return Ok(p);
    }
    let local = ctx.root.join("ghidra-projects");
    if !hidden(&local) {
        return Ok(local);
    }
    let temp = std::env::temp_dir().join("mdx-ghidra-projects");
    if hidden(&temp) || !temp.is_absolute() {
        return Err(invalid(format!(
            "{} has a hidden path component, which Ghidra rejects; set MDX_GHIDRA_PROJECTS to a visible absolute directory",
            ctx.root.display()
        )));
    }
    warnings.push(format!(
        "Project root {} has a hidden path component, which Ghidra rejects; the Ghidra database is under {} (set MDX_GHIDRA_PROJECTS for a persistent location).",
        ctx.root.display(),
        temp.display()
    ));
    Ok(temp)
}

fn command(install: &Installation, attempt: &Path) -> Result<Command> {
    let mut cmd = Command::new(&install.launcher);
    // Keep Ghidra's generated configuration and compiled scripts with the attempt.
    for (key, dir) in [("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache")] {
        let p = attempt.join(dir);
        fs::create_dir_all(&p).map_err(io)?;
        cmd.env(key, p);
    }
    cmd.env("GHIDRA_HEADLESS_MAXMEM", "2G");
    // A perf-data warning on stdout corrupts LaunchSupport's Java-home probe.
    cmd.env("JAVA_TOOL_OPTIONS", format!("{} -XX:-UsePerfData", std::env::var("JAVA_TOOL_OPTIONS").unwrap_or_default()));
    Ok(cmd)
}
fn script_dir(dir: &Path) -> Result<PathBuf> {
    let p = dir.join("scripts");
    fs::create_dir_all(&p).map_err(io)?;
    let file = p.join(SCRIPT_NAME);
    if fs::read_to_string(&file).ok().as_deref() != Some(SCRIPT) {
        fs::write(file, SCRIPT).map_err(io)?;
    }
    Ok(p)
}

// ----------------------------------------------------------------------- import

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Block {
    name: String,
    start: u32,
    size: u32,
    writable: bool,
    #[serde(rename = "volatile")]
    is_volatile: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mapped_to: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
    /// Initial bytes; written beside the request as `file`.
    #[serde(skip)]
    bytes: Option<Vec<u8>>,
}
fn block(name: &str, start: u32, size: u32, writable: bool, is_volatile: bool) -> Block {
    Block { name: name.into(), start, size, writable, is_volatile, mapped_to: None, comment: None, bytes: None }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Mapping {
    cpu: Cpu,
    /// Load address of the imported image (0 for both CPUs).
    base: u32,
    size: u32,
    /// "reset", "state" or "explicit" (m68000 cartridge banks).
    bank_source: String,
    /// ROM offset shown at each 512 KiB slot of the cartridge window (m68000).
    slot_offsets: Vec<u32>,
    z80_rom_offset: Option<u32>,
    z80_origin: Option<u32>,
    code_ranges: Vec<(u32, u32)>,
    entries: Vec<u32>,
    /// Register context applied over all code.
    registers: BTreeMap<String, u32>,
    /// Folded [start, end) ranges marked volatile by request.
    #[serde(default)]
    volatile: Vec<(u32, u32)>,
    blocks: Vec<Block>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Evidence {
    executed: BTreeSet<u32>,
    flows: BTreeSet<(u32, u32)>,
    sources: Vec<Value>,
    ignored_pcs: BTreeMap<String, u64>,
    truncated: bool,
    /// Per address register A0-A6: value -> records (68000 traces only).
    #[serde(skip)]
    address_registers: Vec<BTreeMap<u32, u64>>,
    traced: u64,
}
impl Evidence {
    fn summary(&self) -> Value {
        json!({"sources":self.sources,"executed_pcs":self.executed.len(),"flows":self.flows.len(),"ignored_pcs":self.ignored_pcs,
            "truncated":self.truncated,"register_hints":self.register_hints()})
    }
    /// Address registers holding one value in at least 90% of traced steps: a
    /// candidate base register (compiled code's A5/A4 world) to pass as register context.
    fn register_hints(&self) -> Vec<Value> {
        let mut out = Vec::new();
        for (i, values) in self.address_registers.iter().enumerate() {
            if let Some((&value, &n)) = values.iter().max_by_key(|(_, n)| **n) {
                let share = n as f64 / self.traced.max(1) as f64;
                if self.traced >= 1000 && share >= 0.9 && value != 0 {
                    out.push(json!({"register":format!("A{i}"),"value":format!("0x{value:X}"),"share":(share*1000.0).round()/1000.0,
                        "suggestion":format!("register A{i}={value:X}"),"note":"a hypothesis from executed steps; confirm the code uses it as a base"}));
                }
            }
        }
        out
    }
    fn ignore(&mut self, why: &str) {
        *self.ignored_pcs.entry(why.into()).or_default() += 1;
    }
}

struct Import {
    subject: Subject,
    rom_sha256: String,
    state_sha256: Option<String>,
    payload: Vec<u8>,
    mapping: Mapping,
    data: Vec<Value>,
    labels: Vec<Value>,
    symbols: Vec<Value>,
    evidence: Evidence,
    provenance: Value,
    warnings: Vec<String>,
}

const VECTOR_NAMES: [(usize, &str); 34] = [
    (1, "Reset"), (2, "BusError"), (3, "AddressError"), (4, "IllegalInstruction"), (5, "ZeroDivide"),
    (6, "CHK"), (7, "TRAPV"), (8, "PrivilegeViolation"), (9, "Trace"), (10, "LineA"), (11, "LineF"),
    (24, "SpuriousInterrupt"), (25, "Level1"), (26, "ExtInt"), (27, "Level3"), (28, "HBlank"),
    (29, "Level5"), (30, "VBlank"), (31, "Level7"), (32, "Trap0"), (33, "Trap1"), (34, "Trap2"),
    (35, "Trap3"), (36, "Trap4"), (37, "Trap5"), (38, "Trap6"), (39, "Trap7"), (40, "Trap8"),
    (41, "Trap9"), (42, "Trap10"), (43, "Trap11"), (44, "Trap12"), (45, "Trap13"), (46, "Trap14"),
];
const HARDWARE_68K: [(u32, &str); 24] = [
    (0xA04000, "YM2612_A0"), (0xA04001, "YM2612_D0"), (0xA04002, "YM2612_A1"), (0xA04003, "YM2612_D1"),
    (0xA06000, "Z80_BANK"), (0xA10000, "IO_VERSION"), (0xA10002, "IO_DATA1"), (0xA10004, "IO_DATA2"),
    (0xA10006, "IO_DATA3"), (0xA10008, "IO_CTRL1"), (0xA1000A, "IO_CTRL2"), (0xA1000C, "IO_CTRL3"),
    (0xA11100, "Z80_BUSREQ"), (0xA11200, "Z80_RESET"), (0xA130F1, "SRAM_CTRL"), (0xA14000, "TMSS"),
    (0xC00000, "VDP_DATA"), (0xC00002, "VDP_DATA_MIRROR"), (0xC00004, "VDP_CTRL"), (0xC00006, "VDP_CTRL_MIRROR"),
    (0xC00008, "VDP_HV"), (0xC00011, "PSG"), (0xC0001C, "VDP_DEBUG"), (0xA00000, "Z80_RAM"),
];
const HARDWARE_Z80: [(u32, &str); 6] =
    [(0x4000, "YM2612_A0"), (0x4001, "YM2612_D0"), (0x4002, "YM2612_A1"), (0x4003, "YM2612_D1"), (0x6000, "BANK_REG"), (0x7F11, "PSG")];

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn ranges(specs: &[String]) -> Result<Vec<(u32, u32)>> {
    specs
        .iter()
        .map(|spec| {
            let (lo, hi) = spec.split_once('-').ok_or_else(|| invalid("range must be LO-HI in hex (end exclusive)"))?;
            let (lo, hi) = (hex(lo)?, hex(hi)?);
            if lo >= hi {
                return Err(invalid(format!("empty or reversed range {spec:?}")));
            }
            Ok((lo, hi))
        })
        .collect()
}
/// Merges overlapping/adjacent ranges.
fn merge(mut v: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    v.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (lo, hi) in v {
        match out.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out
}
fn contains(ranges: &[(u32, u32)], a: u32) -> bool {
    ranges.iter().any(|&(lo, hi)| lo <= a && a < hi)
}

fn number(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok()).or_else(|| v.as_str().and_then(|s| hex(s).ok()))
}

/// Executed PCs and observed control transfers from trace/watch JSONL records or a
/// boot result. Records of the other CPU are skipped; nothing is executed here.
fn evidence(input: &Input, ctx: &Context) -> Result<Evidence> {
    let mut ev = Evidence::default();
    let cpu = input.cpu;
    for source in &input.evidence {
        let path = artifacts::resolve(&ctx.store, source)?;
        let bytes = artifacts::read_bounded(&path, FILE_LIMIT)?;
        let (mut records, mut kind) = (0u64, "unknown");
        let before = (ev.executed.len(), ev.flows.len());
        let add = |ev: &mut Evidence, pc: u32, next: Option<u32>| {
            if ev.executed.len() >= MAX_EXECUTED {
                ev.truncated = true;
            } else {
                ev.executed.insert(cpu.fold(pc));
            }
            if let Some(next) = next {
                let (pc, next) = (cpu.fold(pc), cpu.fold(next));
                // Sequential successors carry no control-flow information.
                let sequential = next > pc && next - pc <= 10;
                if !sequential && !(cpu == Cpu::Z80 && matches!(next, 0x38 | 0x66)) {
                    if ev.flows.len() >= MAX_FLOWS {
                        ev.truncated = true;
                    } else {
                        ev.flows.insert((pc, next));
                    }
                }
            }
        };
        if let Ok(doc @ Value::Object(_)) = serde_json::from_slice::<Value>(&bytes) {
            // A boot result envelope (or its data object).
            let data = doc.get("data").filter(|d| d.is_object()).unwrap_or(&doc);
            if data.get("first_addresses").is_none() && data.get("hottest_addresses").is_none() {
                return Err(invalid(format!("{source}: JSON evidence must be a boot result (first/last/hottest addresses)")));
            }
            if cpu == Cpu::Z80 {
                return Err(invalid(format!("{source}: boot results hold 68000 PCs; use a cpu z80 trace for z80 evidence")));
            }
            kind = "boot";
            for key in ["first_addresses", "last_addresses", "hottest_addresses"] {
                for r in data[key].as_array().into_iter().flatten() {
                    if let Some(pc) = number(&r["pc"]) {
                        records += 1;
                        add(&mut ev, pc, number(&r["next_pc"]));
                    }
                }
            }
        } else {
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid(format!("{source}: evidence is neither JSON nor JSONL")))?;
            let mut previous_z80: Option<u32> = None;
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let r: Value = serde_json::from_str(line).map_err(|e| invalid(format!("{source}: invalid JSONL record: {e}")))?;
                records += 1;
                if r["kind"] == "registers" {
                    // Whole-run A0-A6 value frequencies from trace coverage.
                    if cpu == Cpu::M68000 {
                        ev.address_registers.resize(7, BTreeMap::new());
                        ev.traced += r["steps"].as_u64().unwrap_or(0);
                        for (i, values) in r["a"].as_array().into_iter().flatten().take(7).enumerate() {
                            for pair in values.as_array().into_iter().flatten() {
                                if let (Some(v), Some(n)) = (number(&pair[0]), pair[1].as_u64()) {
                                    *ev.address_registers[i].entry(v).or_default() += n;
                                }
                            }
                        }
                    }
                    continue;
                }
                let Some(pc) = number(&r["pc"]) else { continue };
                if matches!(r["kind"].as_str(), Some("pc" | "edge")) {
                    // Trace coverage: distinct PCs and distinct transfers of a whole run.
                    kind = "trace_coverage";
                    if cpu != Cpu::M68000 {
                        ev.ignore("other_cpu");
                        continue;
                    }
                    let next = (r["kind"] == "edge").then(|| number(&r["next_pc"])).flatten();
                    add(&mut ev, pc, next);
                } else if r.get("af").is_some() {
                    // Z80 trace record; successive records are successive instructions
                    // unless a PC filter left gaps (the target check in Ghidra filters most).
                    kind = "z80_trace";
                    if cpu != Cpu::Z80 {
                        ev.ignore("other_cpu");
                        continue;
                    }
                    if let Some(prev) = previous_z80 {
                        add(&mut ev, prev, Some(pc));
                    }
                    add(&mut ev, pc, None);
                    previous_z80 = Some(pc);
                } else if r.get("writer").is_some() {
                    kind = "watch";
                    let z80_writer = r["writer"].as_str().is_some_and(|w| w.to_lowercase().contains("z80"));
                    if z80_writer != (cpu == Cpu::Z80) {
                        ev.ignore("other_cpu");
                        continue;
                    }
                    add(&mut ev, pc, None);
                } else {
                    kind = "trace";
                    if cpu != Cpu::M68000 {
                        ev.ignore("other_cpu");
                        continue;
                    }
                    // An exception step's successor is a vector target, not a flow.
                    let next = if r["exception"].is_null() { number(&r["next_pc"]) } else { None };
                    add(&mut ev, pc, next);
                    if let Some(a) = r["a"].as_array() {
                        ev.address_registers.resize(7, BTreeMap::new());
                        ev.traced += 1;
                        for (i, v) in a.iter().take(7).enumerate() {
                            if let Some(v) = number(v) {
                                *ev.address_registers[i].entry(v).or_default() += 1;
                            }
                        }
                    }
                }
            }
        }
        ev.sources.push(json!({"source":source,"path":path,"sha256":sha256_bytes(&bytes),"kind":kind,"records":records,
            "new_executed_pcs":ev.executed.len()-before.0,"new_flows":ev.flows.len()-before.1}));
    }
    Ok(ev)
}

/// Keeps evidence inside the analysed code; RAM code without a state has no bytes.
fn place_evidence(ev: &mut Evidence, code: &[(u32, u32)], ram_without_bytes: bool) {
    let executed = std::mem::take(&mut ev.executed);
    for pc in executed {
        if contains(code, pc) {
            ev.executed.insert(pc);
        } else if ram_without_bytes && pc >= WRAM {
            ev.ignore("work_ram_without_state");
        } else {
            ev.ignore("outside_analysed_code");
        }
    }
    ev.flows.retain(|&(from, to)| contains(code, from) && contains(code, to));
}

/// RAM ranges covering executed RAM PCs (gaps under 256 bytes are bridged).
fn ram_clusters(pcs: impl Iterator<Item = u32>) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for pc in pcs.filter(|&p| p >= WRAM) {
        let hi = (pc + 10).min(0x100_0000);
        match out.last_mut() {
            Some(last) if pc <= last.1 + 0x100 => last.1 = last.1.max(hi),
            _ => out.push((pc, hi)),
        }
    }
    out
}

fn import(input: &Input, ctx: &Context) -> Result<Import> {
    let rom = input.rom.as_ref().ok_or_else(|| invalid("missing rom"))?;
    let rom_path = artifacts::resolve(&ctx.store, rom)?;
    let cart = Cartridge::load(&rom_path).map_err(|e| Failure {
        code: "rom_unreadable".into(),
        message: e.to_string(),
        remediation: Some("check the path points at a Mega Drive ROM image".into()),
    })?;
    let subject = Subject {
        path: cart.path.display().to_string(),
        sha256: cart.sha256.clone(),
        bytes: cart.data.len(),
        format: "mega-drive-rom".into(),
        architecture: Some(input.cpu.name().into()),
    };
    let state = match &input.from_state {
        Some(s) => {
            let path = artifacts::resolve(&ctx.store, s)?;
            let hash = sha256_bytes(&artifacts::read_bounded(&path, 16 * 1024 * 1024)?);
            let system = super::state::load(cart.clone(), Some(s), super::state::RegionChoice::Auto, ctx)?;
            Some((system, hash, path))
        }
        None => None,
    };
    let mut ev = evidence(input, ctx)?;
    let mut warnings = Vec::new();
    let mut user_ranges = ranges(&input.ranges)?;
    let mut entries = input.entries.iter().map(|s| hex(s).map(|a| input.cpu.fold(a))).collect::<Result<Vec<_>>>()?;
    let mut provenance = json!({"rom":rom_path,"evidence":Value::Null});
    if let Some((_, hash, path)) = &state {
        provenance["state"] = json!({"path":path,"sha256":hash});
    }
    let (mut data, mut labels, mut symbols) = (Vec::new(), Vec::new(), Vec::new());

    let (payload, mapping) = match input.cpu {
        Cpu::Z80 => {
            if !input.banks.is_empty() {
                return Err(invalid("bank applies to cpu m68000"));
            }
            let (image, rom_offset, origin) = match (&state, &input.z80_rom_offset) {
                (Some(_), Some(_)) => return Err(invalid("give z80_rom_offset or from_state, not both")),
                (None, None) => return Err(invalid("cpu z80 needs z80_rom_offset (driver bytes in the ROM) or from_state (its Z80 RAM)")),
                (Some((system, ..)), None) => {
                    if input.z80_origin.is_some() || input.z80_size.is_some() {
                        return Err(invalid("z80_origin and z80_size apply to z80_rom_offset"));
                    }
                    (system.z80_ram.clone(), None, None)
                }
                (None, Some(offset)) => {
                    let offset = hex(offset)?;
                    let origin = input.z80_origin.as_deref().map(hex).transpose()?.unwrap_or(0);
                    if origin >= 0x2000 || offset as usize >= cart.data.len() {
                        return Err(invalid("z80_origin must be below 0x2000 and z80_rom_offset inside the ROM"));
                    }
                    let room = (0x2000 - origin) as usize;
                    let size = input.z80_size.map_or(room.min(cart.data.len() - offset as usize), |s| s as usize);
                    if size == 0 || size > room || offset as usize + size > cart.data.len() {
                        return Err(invalid("z80_size must be 1.. and fit in both the ROM and Z80 RAM after z80_origin"));
                    }
                    let mut image = vec![0u8; 0x2000];
                    image[origin as usize..origin as usize + size].copy_from_slice(&cart.data[offset as usize..offset as usize + size]);
                    warnings.push(format!(
                        "Z80 RAM outside 0x{origin:04X}..0x{:04X} is zero-filled, not uploaded data; a driver that copies or decompresses itself needs from_state.",
                        origin as usize + size
                    ));
                    (image, Some(offset), Some(origin))
                }
            };
            let code = vec![(0, 0x2000)];
            for e in [0x0000u32, 0x0038] {
                if origin.is_none_or(|o| o <= e) && !entries.contains(&e) {
                    entries.push(e);
                }
            }
            if user_ranges.iter().any(|&(lo, hi)| !(lo < 0x2000 && hi <= 0x2000)) {
                return Err(invalid("z80 code ranges must lie in Z80 RAM 0x0000..0x2000"));
            }
            for (a, name) in HARDWARE_Z80 {
                labels.push(json!({"address":a,"name":name}));
            }
            symbols.push(json!({"address":0,"name":"z80_reset","kind":"function","source":"imported"}));
            if entries.contains(&0x38) {
                symbols.push(json!({"address":0x38,"name":"z80_vblank_irq","kind":"function","source":"imported"}));
            }
            let mut mirror = block("Z80_RAM_MIRROR", 0x2000, 0x2000, true, false);
            mirror.mapped_to = Some(0);
            let mut window = block("BANK_WINDOW", 0x8000, 0x8000, false, false);
            window.comment = Some("68000 bus window selected by BANK_REG; contents depend on the bank".into());
            let blocks = vec![
                mirror,
                block("YM2612", 0x4000, 0x2000, true, true),
                block("BANK_REG", 0x6000, 0x1F00, true, true),
                block("VDP_PSG", 0x7F00, 0x100, true, true),
                window,
            ];
            place_evidence(&mut ev, &code, false);
            let mapping = Mapping {
                cpu: Cpu::Z80,
                base: 0,
                size: 0x2000,
                bank_source: "none".into(),
                slot_offsets: vec![],
                z80_rom_offset: rom_offset,
                z80_origin: origin,
                code_ranges: code,
                entries: vec![],
                registers: BTreeMap::new(),
                volatile: vec![],
                blocks,
            };
            (image, mapping)
        }
        Cpu::M68000 => {
            if input.z80_rom_offset.is_some() || input.z80_origin.is_some() || input.z80_size.is_some() {
                return Err(invalid("z80_rom_offset, z80_origin and z80_size apply to cpu z80"));
            }
            let mut slots: BTreeMap<u32, u32> = BTreeMap::new();
            for spec in &input.banks {
                let parse = |s: &str| s.trim().parse::<u32>().map_err(|_| invalid(format!("bank must be SLOT=PAGE in decimal, got {spec:?}")));
                let (slot, page) = spec.split_once('=').ok_or_else(|| invalid("bank must be SLOT=PAGE"))?;
                let (slot, page) = (parse(slot)?, parse(page)?);
                if !(1..=7).contains(&slot) || (page as usize) * 0x8_0000 >= cart.data.len() {
                    return Err(invalid(format!("bank slot must be 1..7 and page inside the ROM ({spec:?})")));
                }
                slots.insert(slot, page);
            }
            if state.is_some() && !slots.is_empty() {
                return Err(invalid("bank overrides conflict with from_state, which carries its own mapping"));
            }
            let fresh;
            let hw: &CartHw = match &state {
                Some((system, ..)) => &system.cart_hw,
                None => {
                    fresh = CartHw::new(&cart);
                    &fresh
                }
            };
            let size = (cart.data.len().div_ceil(2) * 2).min(0x40_0000) as u32;
            let bank_source = if state.is_some() {
                "state"
            } else if slots.is_empty() {
                "reset"
            } else {
                "explicit"
            };
            if cart.data.len() > 0x40_0000 && bank_source == "reset" {
                warnings.push(format!(
                    "ROM is {} bytes but the 68000 sees 4 MiB at a time: analysed with the reset-time bank mapping. Pass bank SLOT=PAGE or a from_state captured in the code of interest to analyse other pages.",
                    cart.data.len()
                ));
            }
            // The 68000's program view: what instruction fetch sees in each bank.
            let byte = |a: u32| -> u8 {
                if let Some(page) = slots.get(&(a >> 19)) {
                    return cart.offset_byte(page * 0x8_0000 + (a & 0x7_FFFF));
                }
                match hw.read_base(a) {
                    Ok(b) => b,
                    Err(offset) => cart.offset_byte(offset),
                }
            };
            let image: Vec<u8> = (0..size).map(byte).collect();
            let slot_offsets = (0..8u32)
                .map(|s| slots.get(&s).map_or_else(|| hw.raw_offset(s * 0x8_0000) & !mdcore::cartridge::RAW_OFFSET, |p| p * 0x8_0000))
                .collect();
            let wram = state.as_ref().map(|(s, ..)| s.wram.clone());
            // Code: the whole ROM window, plus RAM ranges backed by a state's bytes.
            let mut ram = Vec::new();
            for &(lo, hi) in &user_ranges {
                let (flo, fhi) = (input.cpu.fold(lo), input.cpu.fold(hi - 1) + 1);
                if hi <= size {
                    continue;
                }
                if flo < WRAM || fhi <= flo {
                    return Err(invalid(format!("code range {lo:#X}-{hi:#X} is neither in the ROM window nor in work RAM")));
                }
                if wram.is_none() {
                    return Err(invalid("work-RAM code ranges need from_state (a state captured after the code was copied to RAM)"));
                }
                ram.push((flo, fhi));
            }
            if wram.is_some() {
                ram.extend(ram_clusters(ev.executed.iter().copied()));
            }
            // Vector table: functions at every handler, named by vector.
            let mut handlers: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
            let mut outside: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
            if image.len() >= 0x100 {
                for i in 0..64 {
                    let name = VECTOR_NAMES.iter().find(|v| v.0 == i).map(|v| v.1);
                    let label = match (i, name) {
                        (0, _) => "vec_InitialSP".to_string(),
                        (_, Some(n)) => format!("vec_{n}"),
                        _ => format!("vec_Reserved{i}"),
                    };
                    data.push(json!({"address":i*4,"type":"dword","name":label}));
                    let Some(name) = name else { continue };
                    let target = input.cpu.fold(be32(&image, i * 4));
                    if target & 1 != 0 {
                        warnings.push(format!("vector {name} points at odd address {target:#08X}"));
                    } else if target < size && target >= 0x200 {
                        handlers.entry(target).or_default().push(name);
                    } else if target >= WRAM && wram.is_some() {
                        // RAM trampolines (usually JMP abs.l) hold their target only at run time.
                        ram.push((target, (target + 6).min(0x100_0000)));
                        handlers.entry(target).or_default().push(name);
                    } else if target >= WRAM {
                        warnings.push(format!("vector {name} points into work RAM at {target:#08X}; pass from_state to analyse the RAM handler"));
                    } else {
                        outside.entry(target).or_default().push(name);
                    }
                }
            }
            for (target, names) in outside {
                warnings.push(format!("vectors {} point outside the ROM code at {target:#08X}; no handler function created", names.join(", ")));
            }
            let mut used = BTreeSet::new();
            for (target, names) in &handlers {
                let mut name = if names.contains(&"Reset") {
                    "Reset".to_string()
                } else if names.len() >= 3 {
                    "DefaultException".to_string()
                } else {
                    names.join("_")
                };
                while !used.insert(name.clone()) {
                    name.push('_');
                }
                symbols.push(json!({"address":target,"name":name,"kind":"function","source":"imported"}));
            }
            if let Some((&reset, _)) = handlers.iter().find(|(_, n)| n.contains(&"Reset")) {
                entries.insert(0, reset);
            }
            if image.len() >= 0x200 {
                for (a, name, ty, len) in [
                    (0x100, "hdr_system_type", "ascii", 16), (0x110, "hdr_copyright", "ascii", 16),
                    (0x120, "hdr_domestic_title", "ascii", 48), (0x150, "hdr_overseas_title", "ascii", 48),
                    (0x180, "hdr_serial", "ascii", 14), (0x18E, "hdr_checksum", "word", 2),
                    (0x190, "hdr_devices", "ascii", 16), (0x1A0, "hdr_rom_start", "dword", 4),
                    (0x1A4, "hdr_rom_end", "dword", 4), (0x1A8, "hdr_ram_start", "dword", 4),
                    (0x1AC, "hdr_ram_end", "dword", 4), (0x1B0, "hdr_extra_memory", "dword", 4),
                    (0x1B4, "hdr_sram_start", "dword", 4), (0x1B8, "hdr_sram_end", "dword", 4),
                    (0x1BC, "hdr_modem", "ascii", 12), (0x1C8, "hdr_notes", "ascii", 40),
                    (0x1F0, "hdr_regions", "ascii", 16),
                ] {
                    data.push(json!({"address":a,"type":ty,"length":len,"name":name}));
                }
            }
            for (a, name) in HARDWARE_68K {
                labels.push(json!({"address":a,"name":name}));
            }
            let ram = merge(ram);
            let mut code = vec![(0, size)];
            code.extend(&ram);
            place_evidence(&mut ev, &code, wram.is_none());
            if let Some(n) = ev.ignored_pcs.get("work_ram_without_state") {
                warnings.push(format!("{n} executed PCs in work RAM were ignored: pass from_state (captured after the code was copied) to analyse RAM-resident code"));
            }
            let mut blocks = Vec::new();
            if hw.has_sram() && hw.sram_start >= size && hw.sram_end > hw.sram_start && hw.sram_end < 0x40_0000 {
                blocks.push(block("SRAM", hw.sram_start, hw.sram_end - hw.sram_start + 1, true, false));
            }
            let mut z80 = block("Z80_RAM", 0xA0_0000, 0x2000, true, true);
            z80.bytes = state.as_ref().map(|(s, ..)| s.z80_ram.clone());
            let mut work = block("WRAM", WRAM, 0x1_0000, true, false);
            work.bytes = wram;
            let mut short = block("WRAM_ABS_W", 0xFFFF_0000, 0x1_0000, true, false);
            short.mapped_to = Some(WRAM);
            short.comment = Some("Work RAM as sign-extended absolute short addresses ($8000.w = 0xFFFF8000)".into());
            blocks.extend([
                z80,
                block("Z80_BUS", 0xA0_2000, 0xE000, true, true),
                block("IO", 0xA1_0000, 0x1_0000, true, true),
                block("VDP", 0xC0_0000, 0x20, true, true),
                work,
                short,
            ]);
            user_ranges.retain(|&(_, hi)| hi > size);
            let mapping = Mapping {
                cpu: Cpu::M68000,
                base: 0,
                size,
                bank_source: bank_source.into(),
                slot_offsets,
                z80_rom_offset: None,
                z80_origin: None,
                code_ranges: code,
                entries: vec![],
                registers: BTreeMap::new(),
                volatile: vec![],
                blocks,
            };
            (image, mapping)
        }
    };
    let mut mapping = mapping;
    entries.sort_unstable();
    entries.dedup();
    if let Some(e) = entries.iter().find(|&&e| !contains(&mapping.code_ranges, e) || (input.cpu == Cpu::M68000 && e & 1 != 0)) {
        return Err(invalid(format!("entry {e:#X} is not an even address inside the analysed code")));
    }
    mapping.entries = entries;
    for spec in &input.registers {
        let (name, value) = spec.split_once('=').ok_or_else(|| invalid("register must be NAME=HEX"))?;
        let name = name.trim().to_uppercase();
        if name.is_empty() || name.len() > 8 || !name.bytes().all(|b| b.is_ascii_alphanumeric()) || matches!(name.as_str(), "PC" | "SP" | "A7") {
            return Err(invalid(format!("invalid context register {name:?} (PC and the stack pointer cannot be fixed)")));
        }
        mapping.registers.insert(name, hex(value)?);
    }
    mark_volatile(&mut mapping, &input.volatile)?;
    provenance["evidence"] = ev.summary();
    Ok(Import {
        subject,
        rom_sha256: cart.sha256.clone(),
        state_sha256: state.as_ref().map(|(_, h, _)| h.clone()),
        payload,
        mapping,
        data,
        labels,
        symbols,
        evidence: ev,
        provenance,
        warnings,
    })
}

/// Ghidra's decompiler treats memory as volatile per block, so split each
/// block (and every alias block mapped onto it) at the requested ranges and
/// mark the middle pieces volatile.
fn mark_volatile(mapping: &mut Mapping, specs: &[String]) -> Result<()> {
    let cpu = mapping.cpu;
    let mut ranges = Vec::new();
    for spec in specs {
        let (a, len) = match spec.split_once(':') {
            Some((a, l)) => (a, hex(l)?),
            None => (spec.as_str(), 1),
        };
        let start = cpu.fold(hex(a)?);
        if !(1..=0x1_0000).contains(&len) {
            return Err(invalid(format!("volatile {spec:?}: length must be 1..=0x10000")));
        }
        // Each byte must be RAM or hardware that some block models.
        let target = |b: &Block| cpu.fold(b.mapped_to.unwrap_or(b.start));
        let inside = mapping.blocks.iter().any(|b| target(b) <= start && start as u64 + len as u64 <= target(b) as u64 + b.size as u64);
        if !inside {
            return Err(invalid(format!("volatile {spec:?} is not inside one RAM or hardware block (ROM is never volatile)")));
        }
        ranges.push((start, start + len));
    }
    let ranges = merge(ranges);
    if ranges.is_empty() {
        return Ok(());
    }
    let mut blocks = Vec::new();
    for b in std::mem::take(&mut mapping.blocks) {
        let base = cpu.fold(b.mapped_to.unwrap_or(b.start));
        // Cut points as offsets into this block.
        let mut cuts = vec![0, b.size];
        for &(lo, hi) in &ranges {
            for edge in [lo, hi] {
                if edge > base && edge - base < b.size {
                    cuts.push(edge - base);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        if b.is_volatile {
            blocks.push(b);
            continue;
        }
        for (i, w) in cuts.windows(2).enumerate() {
            let (off, size) = (w[0], w[1] - w[0]);
            let mut piece = b.clone();
            piece.start = b.start + off;
            piece.size = size;
            piece.mapped_to = b.mapped_to.map(|m| m + off);
            piece.bytes = b.bytes.as_ref().map(|bytes| bytes[off as usize..(off + size) as usize].to_vec());
            piece.is_volatile = contains(&ranges, base + off);
            if i > 0 {
                piece.name = format!("{}_{:X}", b.name, piece.start);
                piece.comment = None;
            }
            if piece.is_volatile && !b.is_volatile {
                piece.comment = Some("volatile by request".into());
            }
            blocks.push(piece);
        }
    }
    mapping.blocks = blocks;
    mapping.volatile = ranges;
    Ok(())
}

/// Bytes at `addr..addr+len` of the analysed memory (image or a block with bytes).
fn mapped_bytes(payload: &[u8], mapping: &Mapping, addr: u32, len: usize) -> Option<Vec<u8>> {
    let end = addr as usize + len;
    if addr >= mapping.base && end <= (mapping.base + mapping.size) as usize {
        return Some(payload[(addr - mapping.base) as usize..end - mapping.base as usize].to_vec());
    }
    mapping.blocks.iter().find_map(|b| {
        let bytes = b.bytes.as_ref()?;
        (addr >= b.start && end <= (b.start + b.size) as usize)
            .then(|| bytes[(addr - b.start) as usize..end - b.start as usize].to_vec())
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BodyRange {
    start: u32,
    /// Exclusive.
    end: u32,
    sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SymbolFunction {
    address: u32,
    name: String,
    body: Vec<BodyRange>,
}
/// `.symbols.json`: names bound to the exact bytes of each function body.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolsDocument {
    format: u32,
    tool: String,
    cpu: Cpu,
    rom_sha256: String,
    analysis: PathBuf,
    revision: u64,
    functions: Vec<SymbolFunction>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && (b.is_ascii_digit() || b == b'.')))
}

/// Name seeds: body-verified `.symbols.json`, or plain "ADDR NAME" lines.
fn seed_symbols(input: &Input, imp: &mut Import, ctx: &Context) -> Result<()> {
    for source in &input.symbols {
        let path = artifacts::resolve(&ctx.store, source)?;
        let text = String::from_utf8(artifacts::read_bounded(&path, FILE_LIMIT)?).map_err(|_| invalid(format!("{source}: not UTF-8")))?;
        if let Ok(doc) = serde_json::from_str::<SymbolsDocument>(&text) {
            if doc.cpu != input.cpu {
                return Err(invalid(format!("{source} describes {} code", doc.cpu.name())));
            }
            let mut withheld = 0;
            for f in doc.functions {
                let bound = !f.body.is_empty()
                    && f.body.iter().all(|r| {
                        r.end > r.start
                            && mapped_bytes(&imp.payload, &imp.mapping, r.start, (r.end - r.start) as usize)
                                .is_some_and(|b| sha256_bytes(&b) == r.sha256)
                    });
                if bound && valid_name(&f.name) {
                    imp.symbols.push(json!({"address":f.address,"name":f.name,"kind":"function"}));
                } else {
                    withheld += 1;
                }
            }
            if withheld > 0 {
                imp.warnings.push(format!("{source}: {withheld} names withheld because their function bytes differ from this input"));
            }
            continue;
        }
        imp.warnings.push(format!("{source}: plain symbol lines are unverified"));
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            // ADDR NAME [function|data [SIZE]]; without a kind, code addresses are functions.
            let mut words = line.split_whitespace();
            if let (Some(a), Some(name)) = (words.next(), words.next()) {
                let a = input.cpu.fold(hex(a)?);
                if !valid_name(name) {
                    return Err(invalid(format!("{source}: symbol {name:?} is not an identifier")));
                }
                let in_code = contains(&imp.mapping.code_ranges, a);
                let function = match words.next() {
                    None => in_code,
                    Some("function") => true,
                    Some("data") => false,
                    Some(other) => return Err(invalid(format!("{source}: symbol kind {other:?} must be function or data"))),
                };
                if function {
                    if !in_code {
                        return Err(invalid(format!("{source}: function {name:?} at {a:#X} is outside the analysed code")));
                    }
                    imp.symbols.push(json!({"address":a,"name":name,"kind":"function"}));
                } else {
                    let size = words.next().map(|w| w.parse::<u32>().map_err(|_| invalid(format!("{source}: data size {w:?} must be 1, 2 or 4")))).transpose()?;
                    valid_size(size)?;
                    let at = aliases(&imp.mapping, a);
                    if at.is_empty() {
                        return Err(invalid(format!("{source}: data {name:?} at {a:#X} is outside the analysed memory")));
                    }
                    for a in at {
                        imp.symbols.push(json!({"address":a,"name":name,"kind":"data","size":size}));
                    }
                }
            }
        }
    }
    Ok(())
}

// --------------------------------------------------------------------- analysis

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    key: String,
    cpu: Cpu,
    subject: Subject,
    state_sha256: Option<String>,
    /// The exact image Ghidra imported.
    payload: PathBuf,
    payload_sha256: String,
    mapping: Mapping,
    provenance: Value,
    /// Setup request: executed PCs for coverage after annotation.
    setup_request: PathBuf,
    jump_tables: Vec<Value>,
    revision: u64,
    annotations: Vec<Value>,
    installation: Installation,
    language: String,
    script_sha256: String,
    project_dir: PathBuf,
    index: PathBuf,
    project_fingerprint: String,
    warnings: Vec<String>,
}
struct Analysis {
    path: PathBuf,
    manifest: Manifest,
    index: Value,
    fresh_query: Option<Value>,
    _lock: Lock,
}

fn budget(input: &Input) -> Result<Instant> {
    if !(1..=3600).contains(&input.timeout) {
        return Err(invalid("timeout must be 1..=3600 seconds"));
    }
    if input.rom.is_some() == input.analysis.is_some() {
        return Err(invalid("give exactly one of rom or analysis"));
    }
    if input.analysis.is_some()
        && (input.from_state.is_some()
            || !input.evidence.is_empty()
            || !input.ranges.is_empty()
            || !input.entries.is_empty()
            || !input.banks.is_empty()
            || !input.registers.is_empty()
            || !input.volatile.is_empty()
            || !input.symbols.is_empty()
            || input.z80_rom_offset.is_some()
            || input.reanalyze)
    {
        return Err(invalid("import options (from_state, evidence, ranges, entries, banks, registers, volatile, symbols, z80_*, reanalyze) require rom, not an analysis handle"));
    }
    Ok(Instant::now() + Duration::from_secs(input.timeout))
}

fn cached(path: PathBuf, lock: Lock) -> Result<Analysis> {
    let value = read_json(&path)?;
    if value["format"] != FORMAT {
        return Err(fail("stale_analysis", "analysis format changed; query the rom again to create a compatible analysis"));
    }
    let manifest: Manifest = serde_json::from_value(value).map_err(|e| fail("invalid_artifact", e))?;
    if manifest.script_sha256 != sha256_bytes(SCRIPT.as_bytes()) {
        return Err(fail("stale_analysis", "the bundled Ghidra script changed; query the rom again (reanalyze) for a compatible analysis"));
    }
    if !manifest.project_dir.is_dir() {
        return Err(fail("stale_analysis", format!("Ghidra project {} is missing; query the rom with reanalyze", manifest.project_dir.display())));
    }
    if gui_lock(&manifest.project_dir) {
        return Err(fail("analysis_busy", "close this analysis in the Ghidra GUI before querying it"));
    }
    if fingerprint(&manifest.project_dir)? != manifest.project_fingerprint {
        return Err(fail("stale_analysis", "Ghidra project changed outside mdx; cached results are no longer valid. Query the rom with reanalyze for a fresh import; the edited project is preserved."));
    }
    if sha256_bytes(&fs::read(&manifest.payload).map_err(io)?) != manifest.payload_sha256 {
        return Err(fail("invalid_artifact", "analysis input copy has changed"));
    }
    let index = read_json(&manifest.index)?;
    Ok(Analysis { path, manifest, index, fresh_query: None, _lock: lock })
}

fn open_analysis(path: &str, deadline: Instant, ctx: &Context) -> Result<Analysis> {
    let path = PathBuf::from(path);
    let path = if path.is_absolute() { path } else { ctx.root.join(path) };
    let path = path.canonicalize().map_err(|e| invalid(format!("analysis {}: {e}", path.display())))?;
    let lock = Lock::queued(path.parent().ok_or_else(|| invalid("invalid analysis path"))?, deadline, ctx)?;
    cached(path, lock)
}

fn jump_tables(ev: &Evidence) -> Vec<Value> {
    let mut by_source: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for &(from, to) in &ev.flows {
        by_source.entry(from).or_default().insert(to);
    }
    by_source.into_iter().filter(|(_, t)| t.len() >= 2).map(|(from, targets)| json!({"from":from,"targets":targets})).collect()
}

fn prepare(input: &Input, addr: Option<u32>, decompile_timeout: u32, deadline: Instant, ctx: &Context) -> Result<Analysis> {
    if let Some(path) = &input.analysis {
        return open_analysis(path, deadline, ctx);
    }
    let mut imp = import(input, ctx)?;
    seed_symbols(input, &mut imp, ctx)?;
    let install = installation(input.ghidra_home.as_deref())?;
    let language = input.cpu.language();
    let tables = jump_tables(&imp.evidence);
    let identity = json!({"format":FORMAT,"rom":imp.rom_sha256,"state":imp.state_sha256,"payload":sha256_bytes(&imp.payload),
        "ghidra":install.build_sha256,"language":language,"script":sha256_bytes(SCRIPT.as_bytes()),"mapping":imp.mapping,
        "symbols":imp.symbols,"executed":imp.evidence.executed,"flows":imp.evidence.flows});
    let key = sha256_bytes(&serde_json::to_vec(&identity).map_err(io)?);
    let dir = ctx.root.join(".mdx/ghidra").join(&key);
    fs::create_dir_all(&dir).map_err(io)?;
    let dir = dir.canonicalize().map_err(io)?;
    let lock = Lock::queued(&dir, deadline, ctx)?;
    let path = dir.join("analysis.json");
    if path.is_file() && !input.reanalyze {
        return cached(path, lock);
    }
    let attempt = dir.join(stamp());
    fs::create_dir(&attempt).map_err(io)?;
    let project_dir = projects_root(ctx, &mut imp.warnings)?.join(format!("{}-{}", &key[..16], stamp()));
    fs::create_dir_all(&project_dir).map_err(io)?;
    let mut pending = PendingProject(Some(project_dir.clone()));
    let scripts = script_dir(&attempt)?;
    let payload = attempt.join("payload.bin");
    fs::write(&payload, &imp.payload).map_err(io)?;
    let mut blocks = Vec::new();
    for b in &imp.mapping.blocks {
        let mut v = serde_json::to_value(b).map_err(io)?;
        if let Some(bytes) = &b.bytes {
            let file = attempt.join(format!("block-{}.bin", b.name));
            fs::write(&file, bytes).map_err(io)?;
            v["file"] = json!(file);
        }
        blocks.push(v);
    }
    let (main_block, main_writable) = match input.cpu {
        Cpu::M68000 => ("ROM", false),
        Cpu::Z80 => ("Z80_RAM", true),
    };
    let index_path = attempt.join("index.json");
    let setup_result = attempt.join("setup-result.json");
    let setup = attempt.join("setup.json");
    save_json(
        &setup,
        &json!({"mode":"setup","base":imp.mapping.base,"main_block":main_block,"main_writable":main_writable,"blocks":blocks,
            "code_ranges":imp.mapping.code_ranges,"entries":imp.mapping.entries,
            "registers":imp.mapping.registers.iter().map(|(n,v)| json!({"name":n,"value":v})).collect::<Vec<_>>(),"symbols":imp.symbols,"data":imp.data,"labels":imp.labels,
            "executed":imp.evidence.executed,"flows":imp.evidence.flows,"setup_result":setup_result}),
    )?;
    let mut request = json!({"mode":"export","index":index_path,"decompile_timeout":decompile_timeout,"listing_limit":LISTING_LIMIT,
        "query_result":attempt.join("query.json"),"jump_tables":tables,"executed":imp.evidence.executed,
        "clear_data_blocks":imp.mapping.blocks.iter().filter(|b| b.bytes.is_some()).map(|b| b.name.clone()).collect::<Vec<_>>()});
    if let Some(a) = addr {
        request["addr"] = json!(a);
    }
    let export = attempt.join("export.json");
    save_json(&export, &request)?;
    let mut cmd = command(&install, &attempt)?;
    cmd.arg(&project_dir)
        .arg("project")
        .arg("-import")
        .arg(&payload)
        .args(["-loader", "BinaryLoader", "-loader-baseAddr", &format!("0x{:X}", imp.mapping.base), "-processor", language, "-cspec", "default", "-max-cpu", "2"])
        .arg("-analysisTimeoutPerFile")
        .arg(deadline.saturating_duration_since(Instant::now()).as_secs().max(1).to_string())
        .arg("-scriptPath")
        .arg(scripts)
        .arg("-preScript")
        .arg(SCRIPT_NAME)
        .arg(&setup)
        .arg("-postScript")
        .arg(SCRIPT_NAME)
        .arg(export);
    run_worker(&mut cmd, &attempt.join("worker.log"), deadline, ctx)?;
    let log_hint = |e: Failure| fail("ghidra_failed", format!("{}; inspect {}", e.message, attempt.join("worker.log").display()));
    let setup_out = read_json(&setup_result).map_err(log_hint)?;
    let index = read_json(&index_path).map_err(log_hint)?;
    let mut warnings = imp.warnings;
    for w in [&setup_out["warnings"], &index["warnings"]] {
        warnings.extend(w.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned));
    }
    if index["analysis_timed_out"] == true {
        pending.keep();
        return Err(fail(
            "analysis_timeout",
            format!("partial project retained at {}; retry with a larger timeout. It is not cached as complete.", project_dir.display()),
        ));
    }
    imp.provenance["evidence_applied"] = json!({"pcs_rejected":setup_out["evidence_pcs_rejected"],"flows_added":setup_out["evidence_flows_added"],
        "flows_ignored":setup_out["evidence_flows_ignored"],"jump_table_overrides":index["jump_table_overrides"]});
    let manifest = Manifest {
        format: FORMAT,
        key,
        cpu: input.cpu,
        subject: imp.subject,
        state_sha256: imp.state_sha256,
        payload_sha256: sha256_bytes(&imp.payload),
        payload,
        mapping: imp.mapping,
        provenance: imp.provenance,
        setup_request: setup,
        jump_tables: tables,
        revision: 0,
        annotations: vec![],
        installation: install,
        language: language.into(),
        script_sha256: sha256_bytes(SCRIPT.as_bytes()),
        project_fingerprint: fingerprint(&project_dir)?,
        project_dir,
        index: index_path,
        warnings,
    };
    save_json(&path, &manifest)?;
    pending.keep();
    let fresh_query = match addr {
        Some(a) => {
            let value = read_json(&attempt.join("query.json"))?;
            if value["status"] == "complete" {
                save_json(&attempt.join(format!("decompile-{a:08X}.json")), &value)?;
            }
            Some(value)
        }
        None => None,
    };
    Ok(Analysis { path, manifest, index, fresh_query, _lock: lock })
}

// ---------------------------------------------------------------------- results

fn page(rows: &[Value], offset: usize, count: usize) -> Value {
    let items: Vec<&Value> = rows.iter().skip(offset).take(count).collect();
    let end = offset.saturating_add(items.len());
    json!({"total":rows.len(),"offset":offset,"returned":items.len(),"limit":count,"truncated":end<rows.len(),
        "next_offset":(end<rows.len()).then_some(end),"items":items})
}
fn valid_limit(n: usize) -> Result<()> {
    if (1..=1000).contains(&n) {
        Ok(())
    } else {
        Err(invalid("limit must be 1..=1000"))
    }
}
fn references(a: &Analysis) -> &[Value] {
    a.index["references"].as_array().map_or(&[], Vec::as_slice)
}
/// Reference rows with the names of the functions at each end, so a caller
/// reading callees or xrefs needs no second lookup per row: `function_name`
/// for the function containing `from`, `to_name` for a function entered at `to`.
fn named(a: &Analysis, rows: Vec<Value>) -> Vec<Value> {
    let cpu = a.manifest.cpu;
    let names: BTreeMap<u32, &str> = a.index["functions"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|f| Some((cpu.fold(hex(f["entry"].as_str()?).ok()?), f["name"].as_str()?)))
        .collect();
    let name_at = |v: &Value| v.as_str().and_then(|s| hex(s).ok()).and_then(|x| names.get(&cpu.fold(x)).copied());
    rows.into_iter()
        .map(|mut r| {
            if let Some(n) = name_at(&r["function"]) {
                r["function_name"] = json!(n);
            }
            if r["to_space"] == "ram" {
                if let Some(n) = name_at(&r["to"]) {
                    r["to_name"] = json!(n);
                }
            }
            r
        })
        .collect()
}
fn same_address(cpu: Cpu, value: &Value, target: u32) -> bool {
    value.as_str().and_then(|s| hex(s).ok()).is_some_and(|addr| cpu.fold(addr) == target)
}

fn summary(a: &Analysis) -> Value {
    let m = &a.manifest;
    json!({"analysis":a.path,"cpu":m.cpu,"rom_sha256":m.subject.sha256,"state_sha256":m.state_sha256,"ghidra_version":m.installation.version,
        "language":m.language,"project":m.project_dir.join("project.gpr"),"function_count":a.index["functions"].as_array().map_or(0,Vec::len),
        "instructions":a.index["instructions"],"mapping":m.mapping,"provenance":m.provenance,"evidence_coverage":a.index["evidence"],
        "revision":m.revision,"references_truncated":a.index["references_truncated"],
        "base_register_calls":a.index["base_register_calls"]})
}

/// Wraps a result with the analysis' identity, coverage and standing limitations.
fn envelope(tool: &str, a: &Analysis, data: Value, partial: Option<String>) -> Envelope<Value> {
    let m = &a.manifest;
    let functions = a.index["functions"].as_array().map_or(0, Vec::len);
    let mut conditions = vec![format!(
        "Ghidra {} {} analysis {} revision {}: {functions} functions, {} instructions",
        m.installation.version, m.language, &m.key[..16], m.revision, a.index["instructions"]
    )];
    let ev = &a.index["evidence"];
    if ev["executed_pcs"].as_u64().unwrap_or(0) > 0 {
        conditions.push(format!(
            "runtime evidence: {} executed PCs, {} inside analysed functions, {} not disassembled",
            ev["executed_pcs"], ev["in_functions"], ev["not_disassembled"]
        ));
    }
    let base = &a.index["base_register_calls"];
    if base["resolved"].as_u64().unwrap_or(0) > 0 || base["rejected"].as_array().is_some_and(|r| !r.is_empty()) {
        conditions.push(format!(
            "base-register calls (jsr d16(An) with register context): {} resolved, {} rejected (see base_register_calls)",
            base["resolved"], base["rejected"].as_array().map_or(0, Vec::len)
        ));
    }
    let mut coverage = Coverage::complete(conditions);
    if let Some(reason) = partial {
        coverage.status = CoverageStatus::Partial;
        coverage.truncated.push(reason);
    }
    if a.index["references_truncated"] == true {
        coverage.status = CoverageStatus::Partial;
        coverage.truncated.push("references[500000..]".into());
    }
    if m.provenance["evidence"]["truncated"] == true {
        coverage.status = CoverageStatus::Partial;
        coverage.truncated.push(format!("evidence beyond {MAX_EXECUTED} executed PCs / {MAX_FLOWS} flows"));
    }
    coverage.limits = vec![format!("functions/xrefs pages <= 1000 rows; decompile listing <= {LISTING_LIMIT} instructions")];
    coverage.limitations = match m.cpu {
        Cpu::M68000 => vec![
            "Pseudocode, signatures and function extents are Ghidra hypotheses. Hand-written 68000 code passes arguments in arbitrary registers, returns status in CCR flags, and shares tails or falls through between routines; check the listing and runtime evidence before trusting C-like output.".into(),
            "Ghidra's 68000:BE:32:default language decodes a 68040 superset: 68020+ encodings the 68000 traps on can appear as instructions in data.".into(),
            "Indirect jumps/calls are resolved only where Ghidra's analysis or supplied runtime evidence covers them; evidence covers executed paths only.".into(),
            format!("Cartridge window uses the {} bank mapping; work RAM code is analysed only from a supplied state.", m.mapping.bank_source),
        ],
        Cpu::Z80 => vec![
            "Pseudocode and function extents are Ghidra hypotheses for hand-written Z80 sound-driver code; check against the listing and Z80 traces.".into(),
            "The 0x8000 bank window's contents depend on the bank register at run time and are not modelled.".into(),
        ],
    };
    coverage.limitations.push("Static analysis: nothing here executed the game.".into());
    let mut out = Envelope::ok(&format!("mdx.ghidra.{tool}"), Some(m.subject.clone()), coverage, data);
    out.warnings = m.warnings.iter().map(|w| Warning::new("ghidra", w.clone())).collect();
    out
}

pub fn doctor(args: DoctorArgs, ctx: &Context) -> Result<Envelope<Value>> {
    let install = installation(args.ghidra_home.as_deref())?;
    let dir = ctx.root.join(".mdx/ghidra/doctor");
    fs::create_dir_all(&dir).map_err(io)?;
    let log = dir.join(format!("{}.log", stamp()));
    run_worker(Command::new("java").arg("-version"), &log, Instant::now() + Duration::from_secs(15), ctx)
        .map_err(|e| unavailable(format!("java -version failed: {}", e.message)))?;
    let mut warnings = Vec::new();
    let projects = projects_root(ctx, &mut warnings)?;
    let mut coverage = Coverage::complete(vec!["installation files and `java -version` checked; no ROM imported".into()]);
    if install.version != VALIDATED_VERSION {
        coverage.limitations.push(format!("validated with Ghidra {VALIDATED_VERSION}; {} may behave differently", install.version));
    }
    let mut out = Envelope::ok(
        "mdx.ghidra.doctor",
        None,
        coverage,
        json!({"status":"available","ghidra":install,"java":fs::read_to_string(log).map_err(io)?,"validated_version":VALIDATED_VERSION,
            "languages":{"m68000":LANGUAGE_68K,"z80":LANGUAGE_Z80},"projects_root":projects}),
    );
    out.warnings = warnings.into_iter().map(|w| Warning::new("ghidra", w)).collect();
    Ok(out)
}

pub fn analyze(args: AnalyzeArgs, ctx: &Context) -> Result<Envelope<Value>> {
    let input = args.input();
    let deadline = budget(&input)?;
    let a = prepare(&input, None, function_timeout(), deadline, ctx)?;
    Ok(envelope("analyze", &a, summary(&a), None))
}

pub fn functions(args: FunctionsArgs, ctx: &Context) -> Result<Envelope<Value>> {
    valid_limit(args.limit)?;
    let input = args.input();
    let deadline = budget(&input)?;
    let a = prepare(&input, None, function_timeout(), deadline, ctx)?;
    let filter = args.filter.unwrap_or_default().to_lowercase();
    let rows: Vec<Value> = a.index["functions"]
        .as_array()
        .ok_or_else(|| fail("invalid_artifact", "missing functions index"))?
        .iter()
        .filter(|f| f["name"].as_str().unwrap_or("").to_lowercase().contains(&filter))
        .cloned()
        .collect();
    let mut out = summary(&a);
    out["functions"] = page(&rows, args.offset, args.limit);
    Ok(envelope("functions", &a, out, None))
}

pub fn xrefs(args: XrefsArgs, ctx: &Context) -> Result<Envelope<Value>> {
    valid_limit(args.limit)?;
    let input = args.input();
    let deadline = budget(&input)?;
    let a = prepare(&input, None, function_timeout(), deadline, ctx)?;
    let cpu = a.manifest.cpu;
    let target = cpu.fold(hex(&args.addr)?);
    let to: Vec<Value> = references(&a).iter().filter(|r| same_address(cpu, &r["to"], target) && r["to_space"] == "ram").cloned().collect();
    let from: Vec<Value> = references(&a).iter().filter(|r| same_address(cpu, &r["from"], target)).cloned().collect();
    let mut out = summary(&a);
    out["requested_addr"] = json!(args.addr);
    out["addr"] = json!(format!("0x{target:08X}"));
    out["references_to"] = page(&named(&a, to), args.offset, args.limit);
    out["references_from"] = page(&named(&a, from), args.offset, args.limit);
    Ok(envelope("xrefs", &a, out, None))
}

pub fn decompile(args: DecompileArgs, ctx: &Context) -> Result<Envelope<Value>> {
    if !(1..=300).contains(&args.decompile_timeout) {
        return Err(invalid("decompile_timeout must be 1..=300"));
    }
    let input = args.input();
    let deadline = budget(&input)?;
    let requested = hex(&args.addr)?;
    // Before analysis, the CPU comes from the arguments; afterwards from the manifest.
    let mut a = prepare(&input, Some(input.cpu.fold(requested)), args.decompile_timeout, deadline, ctx)?;
    let cpu = a.manifest.cpu;
    let addr = cpu.fold(requested);
    let attempt = a.manifest.index.parent().unwrap().to_path_buf();
    let result_path = attempt.join(format!("decompile-{addr:08X}.json"));
    let mut result = if let Some(result) = a.fresh_query.take() {
        result
    } else if result_path.is_file() {
        read_json(&result_path)?
    } else {
        let install = installation(input.ghidra_home.as_deref().or(Some(&a.manifest.installation.home)))?;
        if install.build_sha256 != a.manifest.installation.build_sha256 {
            return Err(fail("stale_analysis", "Ghidra version changed; query the rom with reanalyze"));
        }
        let request = attempt.join(format!("query-{}.json", stamp()));
        let output = request.with_extension("result.json");
        save_json(
            &request,
            &json!({"mode":"query","addr":addr,"decompile_timeout":args.decompile_timeout,"listing_limit":LISTING_LIMIT,"query_result":output}),
        )?;
        let mut cmd = command(&install, &attempt)?;
        cmd.arg(&a.manifest.project_dir)
            .arg("project")
            .args(["-process", "payload.bin", "-noanalysis", "-readOnly", "-max-cpu", "2"])
            .arg("-scriptPath")
            .arg(script_dir(&attempt)?)
            .arg("-postScript")
            .arg(SCRIPT_NAME)
            .arg(&request);
        run_worker(&mut cmd, &request.with_extension("log"), deadline, ctx)?;
        let result = read_json(&output)
            .map_err(|e| fail("ghidra_failed", format!("{}; inspect {}", e.message, request.with_extension("log").display())))?;
        // Ghidra may touch project metadata even in read-only mode.
        a.manifest.project_fingerprint = fingerprint(&a.manifest.project_dir)?;
        save_json(&a.path, &a.manifest)?;
        if result["status"] == "complete" {
            save_json(&result_path, &result)?;
        }
        result
    };
    let entry = result["function"]["entry"].as_str().unwrap_or("").to_string();
    let entry_addr = hex(&entry).ok().map(|e| cpu.fold(e));
    let callers: Vec<Value> = references(&a)
        .iter()
        .filter(|r| r["call"] == true && r["to_space"] == "ram" && entry_addr.is_some_and(|e| same_address(cpu, &r["to"], e)))
        .cloned()
        .collect();
    let callees: Vec<Value> = references(&a).iter().filter(|r| r["call"] == true && r["function"] == entry).cloned().collect();
    let status = result["status"].as_str().unwrap_or("unknown").to_string();
    let mut out = summary(&a);
    out["requested_addr"] = json!(args.addr);
    out["addr"] = json!(format!("0x{addr:08X}"));
    out["decompile_status"] = json!(status);
    out["function"] = result["function"].take();
    out["pseudocode"] = result["pseudocode"].take();
    out["diagnostic"] = result["diagnostic"].take();
    out["listing"] = result["listing"].take();
    out["listing_truncated"] = result["listing_truncated"].take();
    out["callers"] = page(&named(&a, callers), 0, 100);
    out["callees"] = page(&named(&a, callees), 0, 100);
    out["result_file"] = if result_path.exists() { json!(result_path) } else { Value::Null };
    let mut partial = (status != "complete").then(|| format!("pseudocode (decompile status {status})"));
    if args.brief {
        let o = out.as_object_mut().unwrap();
        for key in ["listing", "listing_truncated", "mapping", "provenance"] {
            o.remove(key);
        }
        o.insert("brief".into(), json!(true));
    } else if out["listing_truncated"] == true {
        partial.get_or_insert_with(|| format!("listing[{LISTING_LIMIT}..]"));
    }
    Ok(envelope("decompile", &a, out, partial))
}

fn patches(cpu: Cpu, index: &Value, patch: Patch) -> Result<(Vec<Value>, Vec<Value>)> {
    if patch.functions.len() > 1000 {
        return Err(invalid("patch allows at most 1000 functions"));
    }
    let functions = index["functions"].as_array().ok_or_else(|| fail("invalid_artifact", "missing functions"))?;
    // Ghidra keys functions by their imported address, which can be an alias.
    let entry_of = |addr: u32| functions.iter().find(|f| same_address(cpu, &f["entry"], addr));
    let mut rows = BTreeMap::new();
    for p in patch.functions {
        let addr = cpu.fold(hex(&p.addr)?);
        let f = entry_of(addr).ok_or_else(|| invalid(format!("0x{addr:08X} is not a function entry")))?;
        let raw = hex(f["entry"].as_str().unwrap_or(""))?;
        if p.name.is_none() && p.comment.is_none() {
            return Err(invalid("empty function patch"));
        }
        let mut row = json!({"address":raw});
        if let Some(name) = p.name {
            if !valid_name(&name) {
                return Err(invalid("names must be ASCII identifiers, at most 128 characters"));
            }
            row["name"] = json!(name);
        }
        if let Some(comment) = p.comment {
            if comment.len() > 16384 || comment.contains('\0') {
                return Err(invalid("comment exceeds 16 KiB or contains NUL"));
            }
            row["comment"] = json!(comment);
        }
        if rows.insert(raw, row).is_some() {
            return Err(invalid("duplicate function in patch"));
        }
    }
    let mut names: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for f in functions {
        let addr = hex(f["entry"].as_str().ok_or_else(|| invalid("invalid indexed entry"))?)?;
        let name = rows.get(&addr).and_then(|r| r["name"].as_str()).unwrap_or_else(|| f["name"].as_str().unwrap_or(""));
        names.entry(name.to_string()).or_default().push(addr);
    }
    let mut conflicts = Vec::new();
    for (name, addrs) in &names {
        if addrs.len() < 2 {
            continue;
        }
        let claimed: Vec<_> = addrs.iter().filter(|a| rows.get(a).is_some_and(|p| p["name"].is_string())).collect();
        if claimed.is_empty() {
            continue;
        }
        if claimed.len() != 1 {
            return Err(fail("symbol_conflict", format!("name {name:?} is ambiguous")));
        }
        for addr in addrs.iter().filter(|a| *a != claimed[0]) {
            let f = functions.iter().find(|f| f["entry"].as_str().and_then(|s| hex(s).ok()) == Some(*addr)).unwrap();
            if matches!(f["name_source"].as_str(), Some("USER_DEFINED" | "IMPORTED")) {
                return Err(fail("symbol_conflict", format!("name {name:?} belongs to user/imported function 0x{addr:08X}")));
            }
            let mut replacement = format!("mdx_generated_{addr:08X}");
            while names.contains_key(&replacement) {
                replacement.push('_');
            }
            let row = rows.entry(*addr).or_insert_with(|| json!({"address":addr}));
            row["name"] = json!(replacement);
            row["generated"] = json!(true);
            conflicts.push(json!({"name":name,"displaced_entry":format!("0x{addr:08X}"),"replacement":replacement}));
        }
    }
    Ok((rows.into_values().collect(), conflicts))
}

/// Label rows for Ghidra, one per alias of each address, so absolute-short
/// work RAM accesses ($E4EE.w) carry the name as well as long ones.
fn label_rows(mapping: &Mapping, labels: Vec<LabelPatch>) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for l in labels {
        let addr = mapping.cpu.fold(hex(&l.addr)?);
        if !valid_name(&l.name) {
            return Err(invalid("names must be ASCII identifiers, at most 128 characters"));
        }
        if let Some(c) = &l.comment {
            if c.len() > 16384 || c.contains('\0') {
                return Err(invalid("comment exceeds 16 KiB or contains NUL"));
            }
        }
        if !seen.insert(addr) {
            return Err(invalid(format!("duplicate label at 0x{addr:08X}")));
        }
        valid_size(l.size)?;
        let at = aliases(mapping, addr);
        if at.is_empty() {
            return Err(invalid(format!("label {:?} at 0x{addr:08X} is outside the analysed memory", l.name)));
        }
        for a in at {
            let mut row = json!({"address":a,"name":l.name});
            if let Some(n) = l.size {
                row["size"] = json!(n);
            }
            if let Some(c) = &l.comment {
                row["comment"] = json!(c);
            }
            rows.push(row);
        }
    }
    Ok(rows)
}

fn valid_size(size: Option<u32>) -> Result<()> {
    match size {
        None | Some(1 | 2 | 4) => Ok(()),
        Some(n) => Err(invalid(format!("data size {n} must be 1, 2 or 4"))),
    }
}

/// Every analysed address (image or block) that shows the folded address `a`.
fn aliases(mapping: &Mapping, a: u32) -> Vec<u32> {
    let cpu = mapping.cpu;
    let mut out = Vec::new();
    if a >= mapping.base && a < mapping.base + mapping.size {
        out.push(a);
    }
    for b in &mapping.blocks {
        let target = cpu.fold(b.mapped_to.unwrap_or(b.start));
        if target <= a && (a as u64) < target as u64 + b.size as u64 {
            out.push(b.start + (a - target));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn copy_project(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir(target).map_err(io)?;
    for entry in fs::read_dir(source).map_err(io)? {
        let entry = entry.map_err(io)?;
        let ty = entry.file_type().map_err(io)?;
        let name = entry.file_name();
        if ty.is_symlink() {
            return Err(fail("invalid_artifact", "project contains a symlink"));
        }
        if name.to_string_lossy().ends_with(".lock") || name.to_string_lossy().ends_with(".lock~") {
            continue;
        }
        let dest = target.join(name);
        if ty.is_dir() {
            copy_project(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest).map_err(io)?;
        }
    }
    Ok(())
}

pub fn annotate(args: AnnotateArgs, ctx: &Context) -> Result<Envelope<Value>> {
    if !(1..=3600).contains(&args.timeout) {
        return Err(invalid("timeout must be 1..=3600"));
    }
    let inline = !args.functions.is_empty() || !args.labels.is_empty();
    let mut patch: Patch = match (&args.file, inline) {
        (Some(file), false) => {
            let file = if file.is_absolute() { file.clone() } else { ctx.root.join(file) };
            serde_json::from_value(read_json(&file)?).map_err(invalid)?
        }
        (None, true) => Patch { functions: args.functions.clone(), labels: args.labels.clone() },
        _ => return Err(invalid("give exactly one of file or functions/labels")),
    };
    if !(1..=1000).contains(&(patch.functions.len() + patch.labels.len())) {
        return Err(invalid("patch requires 1..=1000 functions and labels"));
    }
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    let mut a = open_analysis(&args.analysis, deadline, ctx)?;
    let labels = label_rows(&a.manifest.mapping, std::mem::take(&mut patch.labels))?;
    let (patches, conflicts) = patches(a.manifest.cpu, &a.index, patch)?;
    let install = installation(args.ghidra_home.as_deref().or(Some(&a.manifest.installation.home)))?;
    if install.build_sha256 != a.manifest.installation.build_sha256 {
        return Err(fail("stale_analysis", "Ghidra version changed"));
    }
    // Work on a copy: timeout, cancellation or script failure cannot leave the
    // published manifest pointing at a partially annotated database.
    let attempt = a.path.parent().unwrap().join(stamp());
    fs::create_dir(&attempt).map_err(io)?;
    let project = a.manifest.project_dir.with_file_name(format!("{}-{}", &a.manifest.key[..16], stamp()));
    let mut pending = PendingProject(Some(project.clone()));
    copy_project(&a.manifest.project_dir, &project)?;
    let index = attempt.join("index.json");
    let request = attempt.join("annotations.json");
    let executed = read_json(&a.manifest.setup_request).map(|s| s["executed"].clone()).unwrap_or(json!([]));
    save_json(&request, &json!({"mode":"annotate","patches":patches,"labels":labels,"index":index,"executed":executed}))?;
    let mut cmd = command(&install, &attempt)?;
    cmd.arg(&project)
        .arg("project")
        .args(["-process", "payload.bin", "-noanalysis", "-max-cpu", "2"])
        .arg("-scriptPath")
        .arg(script_dir(&attempt)?)
        .arg("-postScript")
        .arg(SCRIPT_NAME)
        .arg(&request);
    run_worker(&mut cmd, &attempt.join("worker.log"), deadline, ctx)?;
    let mut fresh = read_json(&index)?;
    if !fresh["functions"].is_array() {
        return Err(fail("ghidra_failed", "annotation index missing"));
    }
    // Annotation runs without the evidence request; keep the import's override count.
    fresh["jump_table_overrides"] = a.index["jump_table_overrides"].clone();
    fresh["base_register_calls"] = a.index["base_register_calls"].clone();
    save_json(&index, &fresh)?;
    a.index = fresh;
    a.manifest.revision += 1;
    a.manifest.annotations.push(json!({"revision":a.manifest.revision,"patch":request,"conflicts":conflicts}));
    a.manifest.index = index;
    let old = std::mem::replace(&mut a.manifest.project_dir, project);
    let old_fingerprint = std::mem::replace(&mut a.manifest.project_fingerprint, fingerprint(&a.manifest.project_dir)?);
    save_json(&a.path, &a.manifest)?;
    pending.keep();
    discard_project(&old, Some(&old_fingerprint));
    let mut out = summary(&a);
    out["conflicts"] = json!(conflicts);
    out["patches"] = json!(patches);
    out["labels"] = json!(labels);
    Ok(envelope("annotate", &a, out, None))
}

pub fn export_symbols(args: ExportArgs, ctx: &Context) -> Result<Envelope<Value>> {
    if !(1..=3600).contains(&args.timeout) {
        return Err(invalid("timeout must be 1..=3600"));
    }
    let a = open_analysis(&args.analysis, Instant::now() + Duration::from_secs(args.timeout), ctx)?;
    let m = &a.manifest;
    let payload = fs::read(&m.payload).map_err(io)?;
    // Block bytes (e.g. a state's work RAM) live beside the setup request.
    let mut mapping = m.mapping.clone();
    let attempt = m.setup_request.parent().unwrap_or(Path::new("."));
    for b in &mut mapping.blocks {
        let file = attempt.join(format!("block-{}.bin", b.name));
        if file.is_file() {
            b.bytes = Some(fs::read(file).map_err(io)?);
        }
    }
    let mut functions = Vec::new();
    let mut names = BTreeSet::new();
    let mut unbound = Vec::new();
    for f in a.index["functions"].as_array().ok_or_else(|| fail("invalid_artifact", "missing functions"))? {
        let address = m.cpu.fold(hex(f["entry"].as_str().ok_or_else(|| invalid("invalid indexed entry"))?)?);
        let mut body = Vec::new();
        for r in f["body"].as_array().ok_or_else(|| invalid("missing function body"))? {
            let start = m.cpu.fold(hex(r["start"].as_str().ok_or_else(|| invalid("invalid body start"))?)?);
            let end = m.cpu.fold(hex(r["end_inclusive"].as_str().ok_or_else(|| invalid("invalid body end"))?)?) + 1;
            match (end > start).then(|| mapped_bytes(&payload, &mapping, start, (end - start) as usize)).flatten() {
                Some(bytes) => body.push(BodyRange { start, end, sha256: sha256_bytes(&bytes) }),
                None => {
                    body.clear();
                    break;
                }
            }
        }
        let name = f["name"].as_str().ok_or_else(|| invalid("missing function name"))?;
        if body.is_empty() {
            unbound.push(name.to_string());
            continue;
        }
        if !names.insert(name.to_owned()) {
            return Err(fail("symbol_conflict", format!("export requires unique names; rename the functions called {name:?} first")));
        }
        functions.push(SymbolFunction { address, name: name.into(), body });
    }
    let document = SymbolsDocument {
        format: 1,
        tool: "mdx.ghidra".into(),
        cpu: m.cpu,
        rom_sha256: m.subject.sha256.clone(),
        analysis: a.path.clone(),
        revision: m.revision,
        functions,
    };
    let path = match &args.out {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => ctx.root.join(p),
        None => m.index.parent().unwrap().join("export.symbols.json"),
    };
    if !path.to_string_lossy().ends_with(".symbols.json") {
        return Err(invalid("output must end in .symbols.json"));
    }
    let bytes = serde_json::to_vec_pretty(&document).map_err(io)?;
    if path.exists() {
        if read_json(&path)? != serde_json::to_value(&document).map_err(io)? {
            return Err(fail("output_exists", "symbol output exists with different content; choose a new path"));
        }
    } else {
        fs::write(&path, &bytes).map_err(io)?;
    }
    let artifact = artifacts::put(&ctx.store, &bytes)?;
    let data = json!({"analysis":a.path,"revision":m.revision,"symbols":path,"symbols_artifact":artifact,"function_count":document.functions.len(),
        "unbound_functions":unbound,"verification":"every exported body range is bound to its bytes by SHA-256; names are withheld on import where bytes differ"});
    let partial = (!unbound.is_empty()).then(|| format!("{} functions whose bodies lie outside captured bytes", unbound.len()));
    Ok(envelope("export_symbols", &a, data, partial))
}

/// Dispatch for operations::execute.
pub fn run(name: &str, arguments: Value, ctx: &Context) -> Result<Envelope<Value>> {
    fn de<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
        serde_json::from_value(v).map_err(invalid)
    }
    match name {
        "ghidra.doctor" => doctor(de(arguments)?, ctx),
        "ghidra.analyze" => analyze(de(arguments)?, ctx),
        "ghidra.functions" => functions(de(arguments)?, ctx),
        "ghidra.decompile" => decompile(de(arguments)?, ctx),
        "ghidra.xrefs" => xrefs(de(arguments)?, ctx),
        "ghidra.annotate" => annotate(de(arguments)?, ctx),
        "ghidra.export_symbols" => export_symbols(de(arguments)?, ctx),
        _ => Err(fail("usage", format!("unknown operation {name:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> (tempfile::TempDir, Context) {
        let dir = tempfile::Builder::new().prefix("mdx-ghidra-unit").tempdir().unwrap();
        let ctx = Context { root: dir.path().to_path_buf(), store: dir.path().join("store"), ..Context::default() };
        (dir, ctx)
    }
    /// Reset at 0x200 calls 0x220 and an indirect target; HBlank/VBlank share 0x250.
    fn rom(dir: &Path) -> PathBuf {
        let mut b = vec![0u8; 0x400];
        b[0..4].copy_from_slice(&0x00FF_1000u32.to_be_bytes());
        b[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        b[0x70..0x74].copy_from_slice(&0x250u32.to_be_bytes());
        b[0x78..0x7C].copy_from_slice(&0x250u32.to_be_bytes());
        b[0x68..0x6C].copy_from_slice(&0xFFFF_0100u32.to_be_bytes());
        b[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
        let code: &[(usize, &[u8])] = &[
            (0x200, &[0x4E, 0xB9, 0, 0, 0x02, 0x20, 0x20, 0x79, 0xFF, 0xFF, 0, 0, 0x4E, 0x90, 0x60, 0xFE]),
            (0x220, &[0x30, 0x3C, 0, 5, 0xD0, 0x41, 0x33, 0xC0, 0, 0xFF, 0x80, 0, 0x4E, 0x75]),
            (0x240, &[0x70, 0x01, 0x4E, 0x75]),
            (0x250, &[0x4E, 0x73]),
        ];
        for (at, bytes) in code {
            b[*at..*at + bytes.len()].copy_from_slice(bytes);
        }
        let path = dir.join("game.md");
        fs::write(&path, b).unwrap();
        path
    }
    fn input(rom: &Path) -> Input {
        Input { rom: Some(rom.display().to_string()), timeout: 300, ghidra_home: Some(PathBuf::from("/nonexistent/ghidra")), ..Input::default() }
    }

    #[test]
    fn addresses_fold_like_the_bus() {
        assert_eq!(Cpu::M68000.fold(0xFFFF_8000), 0xFF_8000);
        assert_eq!(Cpu::M68000.fold(0x00E0_1234), 0xFF_1234);
        assert_eq!(Cpu::M68000.fold(0x0100_0200), 0x200);
        assert_eq!(Cpu::M68000.fold(0xC0_0004), 0xC0_0004);
        assert_eq!(Cpu::Z80.fold(0x1_0038), 0x38);
        assert!(same_address(Cpu::M68000, &json!("0xFFFF8000"), 0xFF_8000));
        assert_eq!(hex("$1A2C").unwrap(), 0x1A2C);
        assert_eq!(hex("0x00FF0000").unwrap(), 0xFF_0000);
        assert!(hex("zz").is_err());
    }

    /// Plain symbol lines name data as well as functions; a WRAM name also
    /// lands on the absolute-short alias.
    #[test]
    fn plain_symbol_lines_name_data_and_its_aliases() {
        let (dir, ctx) = context();
        let mut inp = input(&rom(dir.path()));
        fs::write(dir.path().join("game.sym"), "220 Helper\nFFE800 player_x data 2\n300 table data\n").unwrap();
        inp.symbols = vec![dir.path().join("game.sym").display().to_string()];
        let mut imp = import(&inp, &ctx).unwrap();
        seed_symbols(&inp, &mut imp, &ctx).unwrap();
        let mut data: Vec<(u64, &str, &Value)> = imp.symbols.iter().filter(|s| s["kind"] == "data")
            .map(|s| (s["address"].as_u64().unwrap(), s["name"].as_str().unwrap(), &s["size"])).collect();
        data.sort_by_key(|d| d.0);
        assert_eq!(data, [(0x300, "table", &Value::Null), (0xFF_E800, "player_x", &json!(2)), (0xFFFF_E800, "player_x", &json!(2))]);
        assert!(imp.symbols.iter().any(|s| s["name"] == "Helper" && s["kind"] == "function"));
        for bad in ["FFE800 ram_fn function\n", "FFE800 x data 3\n", "FFE800 x thing\n"] {
            fs::write(dir.path().join("game.sym"), bad).unwrap();
            let mut imp = import(&inp, &ctx).unwrap();
            assert!(seed_symbols(&inp, &mut imp, &ctx).is_err(), "{bad}");
        }
        let rows = label_rows(&imp.mapping, vec![LabelPatch { addr: "FFFFE800".into(), name: "low".into(), size: Some(2), comment: None }]).unwrap();
        assert_eq!(rows.iter().map(|r| r["address"].as_u64().unwrap()).collect::<Vec<_>>(), [0xFF_E800, 0xFFFF_E800]);
        assert!(label_rows(&imp.mapping, vec![LabelPatch { addr: "800000".into(), name: "x".into(), size: None, comment: None }]).is_err());
    }

    /// A polled counter is volatile in WRAM and in its absolute-short alias;
    /// the rest of RAM stays ordinary so stores still fold.
    #[test]
    fn volatile_ranges_split_ram_and_its_alias() {
        let (dir, ctx) = context();
        let mut inp = input(&rom(dir.path()));
        inp.volatile = vec!["FFFFE4EE:4".into(), "FFE472".into()];
        let imp = import(&inp, &ctx).unwrap();
        assert_eq!(imp.mapping.volatile, vec![(0xFF_E472, 0xFF_E473), (0xFF_E4EE, 0xFF_E4F2)]);
        let vol: Vec<(&str, u32, u32)> = imp.mapping.blocks.iter().filter(|b| b.is_volatile && b.name.starts_with("WRAM"))
            .map(|b| (b.name.as_str(), b.start, b.size)).collect();
        assert_eq!(vol, [("WRAM_FFE472", 0xFF_E472, 1), ("WRAM_FFE4EE", 0xFF_E4EE, 4),
            ("WRAM_ABS_W_FFFFE472", 0xFFFF_E472, 1), ("WRAM_ABS_W_FFFFE4EE", 0xFFFF_E4EE, 4)]);
        let alias = imp.mapping.blocks.iter().find(|b| b.name == "WRAM_ABS_W_FFFFE4EE").unwrap();
        assert_eq!(alias.mapped_to, Some(0xFF_E4EE));
        let total: u32 = imp.mapping.blocks.iter().filter(|b| b.name.starts_with("WRAM_ABS_W")).map(|b| b.size).sum();
        assert_eq!(total, 0x1_0000);
        for bad in ["200", "FFE4EE:0", "FFFFF0:20"] {
            inp.volatile = vec![bad.into()];
            assert!(import(&inp, &ctx).is_err(), "{bad}");
        }
    }

    #[test]
    fn vectors_header_hardware_and_banks_are_mapped_before_ghidra_runs() {
        let (dir, ctx) = context();
        let imp = import(&input(&rom(dir.path())), &ctx).unwrap();
        assert_eq!(imp.mapping.size, 0x400);
        assert_eq!(imp.mapping.code_ranges, vec![(0, 0x400)]);
        assert_eq!(imp.mapping.entries, vec![0x200]);
        assert_eq!(imp.mapping.bank_source, "reset");
        let named: BTreeMap<u64, &str> = imp.symbols.iter().map(|s| (s["address"].as_u64().unwrap(), s["name"].as_str().unwrap())).collect();
        assert_eq!(named[&0x200], "Reset");
        assert_eq!(named[&0x250], "HBlank_VBlank");
        assert!(imp.warnings.iter().any(|w| w.contains("ExtInt points into work RAM")), "{:?}", imp.warnings);
        assert!(imp.warnings.iter().any(|w| w.contains("BusError") && w.contains("outside the ROM code")));
        assert!(imp.data.iter().any(|d| d["name"] == "vec_VBlank" && d["address"] == 0x78));
        assert!(imp.data.iter().any(|d| d["name"] == "hdr_checksum"));
        assert!(imp.labels.iter().any(|l| l["name"] == "VDP_CTRL" && l["address"] == 0xC00004));
        let names: Vec<&str> = imp.mapping.blocks.iter().map(|b| b.name.as_str()).collect();
        // mdcore, like the reference, gives header-less small carts backup RAM at 0x200000.
        assert_eq!(names, ["SRAM", "Z80_RAM", "Z80_BUS", "IO", "VDP", "WRAM", "WRAM_ABS_W"]);
        assert_eq!(imp.mapping.blocks[0].start, 0x20_0000);
        let short = &imp.mapping.blocks[6];
        assert_eq!((short.start, short.mapped_to), (0xFFFF_0000, Some(WRAM)));
        assert!(imp.mapping.blocks.iter().all(|b| b.bytes.is_none()), "no state: RAM is uninitialised");
        assert_eq!(imp.payload[0x200..0x202], [0x4E, 0xB9]);
        // Explicit SSF2-style banks are bounded by the ROM.
        let mut banked = input(&rom(dir.path()));
        banked.banks = vec!["1=4".into()];
        assert_eq!(import(&banked, &ctx).err().unwrap().code, "invalid_arguments");
        banked.banks = vec!["8=0".into()];
        assert!(import(&banked, &ctx).is_err());
    }

    #[test]
    fn runtime_evidence_seeds_code_flows_and_register_hints() {
        let (dir, ctx) = context();
        let rom = rom(dir.path());
        let mut lines = String::new();
        let mut record = |pc: u32, next: u32, a5: u32| {
            lines.push_str(&json!({"pc":pc,"next_pc":next,"exception":null,"a":[0,0,0,0,0,a5,0,0xFF1000]}).to_string());
            lines.push('\n');
        };
        for _ in 0..600 {
            record(0x20C, 0x240, 0x312);
            record(0x240, 0x242, 0x312);
            record(0x226, 0x22C, 0x312); // sequential: no flow
        }
        record(0xFF_0100, 0xFF_0102, 0); // RAM code without a state
        let trace = dir.path().join("trace.jsonl");
        fs::write(&trace, lines).unwrap();
        let mut a = input(&rom);
        a.evidence = vec![trace.display().to_string()];
        let imp = import(&a, &ctx).unwrap();
        assert_eq!(imp.evidence.executed.iter().copied().collect::<Vec<_>>(), [0x20C, 0x226, 0x240]);
        assert_eq!(imp.evidence.flows.iter().copied().collect::<Vec<_>>(), [(0x20C, 0x240)]);
        assert_eq!(imp.evidence.ignored_pcs["work_ram_without_state"], 1);
        assert!(imp.warnings.iter().any(|w| w.contains("1 executed PCs in work RAM were ignored")));
        let hints = imp.evidence.register_hints();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0]["suggestion"], "register A5=312");
        // Records of the other CPU are not evidence for this one.
        a.cpu = Cpu::Z80;
        a.z80_rom_offset = Some("200".into());
        let z80 = import(&a, &ctx).unwrap();
        assert!(z80.evidence.executed.is_empty());
        assert!(z80.evidence.ignored_pcs["other_cpu"] > 0);
    }

    #[test]
    fn trace_coverage_artifacts_are_evidence() {
        let (dir, ctx) = context();
        let rom = rom(dir.path());
        let rows = [
            json!({"kind":"pc","pc":0x20C,"hits":5,"first_step":3,"first_frame":0}),
            json!({"kind":"pc","pc":0x240,"hits":5,"first_step":4,"first_frame":0}),
            json!({"kind":"edge","pc":0x20C,"next_pc":0x240,"hits":5}),
            json!({"kind":"registers","steps":2000,"a":[[],[],[],[],[],[[0x312,1990],[0,10]],[]]}),
        ];
        let path = dir.path().join("coverage.jsonl");
        fs::write(&path, rows.iter().map(|r| r.to_string() + "\n").collect::<String>()).unwrap();
        let mut a = input(&rom);
        a.evidence = vec![path.display().to_string()];
        let imp = import(&a, &ctx).unwrap();
        assert_eq!(imp.evidence.executed.iter().copied().collect::<Vec<_>>(), [0x20C, 0x240]);
        assert_eq!(imp.evidence.flows.iter().copied().collect::<Vec<_>>(), [(0x20C, 0x240)]);
        assert_eq!(imp.evidence.sources[0]["kind"], "trace_coverage");
        assert_eq!(imp.evidence.register_hints()[0]["suggestion"], "register A5=312");
    }

    #[test]
    fn z80_images_come_from_the_rom_or_a_state() {
        let (dir, ctx) = context();
        let rom = rom(dir.path());
        let mut a = input(&rom);
        a.cpu = Cpu::Z80;
        assert_eq!(import(&a, &ctx).err().unwrap().code, "invalid_arguments");
        a.z80_rom_offset = Some("200".into());
        a.z80_origin = Some("100".into());
        a.z80_size = Some(0x10);
        let imp = import(&a, &ctx).unwrap();
        assert_eq!(imp.payload.len(), 0x2000);
        assert_eq!(imp.payload[0x100..0x102], [0x4E, 0xB9]);
        assert_eq!(imp.payload[0], 0);
        assert_eq!(imp.mapping.entries, Vec::<u32>::new(), "nothing uploaded at the reset/IRQ entries");
        assert!(imp.warnings.iter().any(|w| w.contains("zero-filled")));
        a.z80_origin = None;
        a.z80_size = None;
        let imp = import(&a, &ctx).unwrap();
        assert_eq!(imp.mapping.entries, vec![0, 0x38]);
        assert!(imp.labels.iter().any(|l| l["name"] == "YM2612_A0" && l["address"] == 0x4000));
        a.z80_size = Some(0x2001);
        assert!(import(&a, &ctx).is_err());
    }

    #[test]
    fn validation_and_missing_ghidra_fail_before_any_project_is_created() {
        let (dir, ctx) = context();
        let rom = rom(dir.path());
        let run = |a: Input| prepare(&a, None, 30, budget(&a)?, &ctx).map(|_| ());
        let mut both = input(&rom);
        both.analysis = Some("analysis.json".into());
        assert_eq!(budget(&both).err().unwrap().code, "invalid_arguments");
        let mut handle = Input { analysis: Some("analysis.json".into()), timeout: 300, ..Input::default() };
        handle.evidence = vec!["trace.jsonl".into()];
        assert_eq!(budget(&handle).err().unwrap().code, "invalid_arguments");
        let mut ranges = input(&rom);
        ranges.ranges = vec!["FF0000-FF0100".into()];
        assert!(run(ranges).err().unwrap().message.contains("from_state"));
        let mut entry = input(&rom);
        entry.entries = vec!["201".into()];
        assert_eq!(run(entry).err().unwrap().code, "invalid_arguments");
        let missing = run(input(&rom)).err().unwrap();
        assert_eq!(missing.code, "ghidra_unavailable");
        assert!(missing.remediation.unwrap().contains("MDX_GHIDRA_HOME"));
        assert!(!dir.path().join(".mdx/ghidra").exists(), "failures must not create analyses");
        assert!(!dir.path().join("ghidra-projects").exists());
    }

    #[test]
    fn symbol_seeds_are_byte_verified() {
        let (dir, ctx) = context();
        let rom = rom(dir.path());
        let mut imp = import(&input(&rom), &ctx).unwrap();
        let body = |bytes: &[u8]| vec![BodyRange { start: 0x240, end: 0x244, sha256: sha256_bytes(bytes) }];
        let doc = |functions| SymbolsDocument {
            format: 1, tool: "mdx.ghidra".into(), cpu: Cpu::M68000, rom_sha256: String::new(), analysis: PathBuf::new(), revision: 0, functions,
        };
        let path = dir.path().join("seed.symbols.json");
        save_json(&path, &doc(vec![
            SymbolFunction { address: 0x240, name: "ReturnOne".into(), body: body(&[0x70, 0x01, 0x4E, 0x75]) },
            SymbolFunction { address: 0x240, name: "Stale".into(), body: body(&[0x70, 0x02, 0x4E, 0x75]) },
        ])).unwrap();
        let plain = dir.path().join("names.sym");
        fs::write(&plain, "; comment\n220 AddFive\n").unwrap();
        let mut a = input(&rom);
        a.symbols = vec![path.display().to_string(), plain.display().to_string()];
        seed_symbols(&a, &mut imp, &ctx).unwrap();
        let names: Vec<&str> = imp.symbols.iter().filter_map(|s| s["name"].as_str()).collect();
        assert!(names.contains(&"ReturnOne") && names.contains(&"AddFive") && !names.contains(&"Stale"), "{names:?}");
        assert!(imp.warnings.iter().any(|w| w.contains("1 names withheld")));
        assert!(imp.warnings.iter().any(|w| w.contains("unverified")));
    }

    #[test]
    fn annotations_prioritize_user_names_and_report_collisions() {
        let index = json!({"functions":[{"entry":"0x00000200","name":"FUN_00000200","name_source":"DEFAULT"},
            {"entry":"0x00000240","name":"named","name_source":"USER_DEFINED"},
            {"entry":"0xFFFF0100","name":"FUN_ffff0100","name_source":"DEFAULT"}]});
        let patch = |v| serde_json::from_value(v).unwrap();
        let (rows, conflicts) = patches(Cpu::M68000, &index, patch(json!({"functions":[{"addr":"240","name":"FUN_00000200","comment":"observed"}]}))).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(rows[0]["generated"], true);
        let err = patches(Cpu::M68000, &index, patch(json!({"functions":[{"addr":"200","name":"named"}]}))).err().unwrap();
        assert_eq!(err.code, "symbol_conflict");
        assert!(patches(Cpu::M68000, &index, patch(json!({"functions":[{"addr":"202","name":"interior"}]}))).is_err());
        // A work-RAM alias names the function Ghidra keys at 0xFFFF0100.
        let (rows, _) = patches(Cpu::M68000, &index, patch(json!({"functions":[{"addr":"FF0100","name":"RamCode"}]}))).unwrap();
        assert_eq!(rows[0]["address"], 0xFFFF_0100u32);
        let (rows, _) = patches(Cpu::M68000, &index, patch(json!({"functions":[{"addr":"200","name":"named"},{"addr":"240","name":"FUN_00000200"}]}))).unwrap();
        assert_eq!(rows.len(), 2); // name swaps are valid
    }

    #[test]
    fn paging_and_project_locations_are_bounded() {
        let rows = vec![json!(1), json!(2), json!(3)];
        assert_eq!(page(&rows, 1, 1), json!({"total":3,"offset":1,"returned":1,"limit":1,"truncated":true,"next_offset":2,"items":[2]}));
        assert_eq!(page(&rows, usize::MAX, 1)["items"], json!([]));
        let (dir, mut ctx) = context();
        let mut warnings = vec![];
        assert_eq!(projects_root(&ctx, &mut warnings).unwrap(), dir.path().join("ghidra-projects"));
        ctx.root = dir.path().join(".hidden/project");
        let fallback = projects_root(&ctx, &mut warnings).unwrap();
        assert!(!fallback.starts_with(&ctx.root));
        assert!(warnings[0].contains("hidden path component"));
    }

    #[test]
    fn project_locks_are_exclusive_and_edits_are_detected() {
        let (dir, ctx) = context();
        let first = Lock::acquire(&ctx.root).unwrap();
        assert_eq!(Lock::acquire(&ctx.root).err().unwrap().code, "analysis_busy");
        drop(first);
        let held = Lock::acquire(&ctx.root).unwrap();
        assert_eq!(Lock::queued(&ctx.root, Instant::now() + Duration::from_millis(75), &ctx).err().unwrap().code, "timeout");
        drop(held);
        let before = fingerprint(&ctx.root).unwrap();
        fs::write(ctx.root.join("database"), "modified").unwrap();
        assert_ne!(before, fingerprint(&ctx.root).unwrap());
        fs::write(ctx.root.join("project.lock"), "GUI owner").unwrap();
        assert!(gui_lock(&ctx.root));
        fs::write(ctx.root.join("mdx.lock"), "foreign owner").unwrap();
        assert_eq!(Lock::queued(&ctx.root, Instant::now() + Duration::from_secs(2), &ctx).err().unwrap().code, "analysis_busy");
        drop(dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancellation_and_timeout_stop_worker_descendants() {
        for stop in [false, true] {
            let (dir, ctx) = context();
            let pid_file = ctx.root.join("child.pid");
            let watch = pid_file.clone();
            let token = ctx.cancellation.clone();
            let cancel = std::thread::spawn(move || {
                for _ in 0..100 {
                    if watch.exists() {
                        if stop {
                            token.cancel();
                        }
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "mdx-worker-test"]).arg(&pid_file);
            let failure = run_worker(&mut cmd, &ctx.root.join("worker.log"), Instant::now() + Duration::from_millis(500), &ctx).unwrap_err();
            cancel.join().unwrap();
            assert_eq!(failure.code, if stop { "cancelled" } else { "timeout" });
            let pid = fs::read_to_string(pid_file).unwrap();
            let gone = || fs::read_to_string(format!("/proc/{}/stat", pid.trim())).map_or(true, |s| s.split_whitespace().nth(2) == Some("Z"));
            for _ in 0..100 {
                if gone() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(gone(), "worker descendant still running");
            drop(dir);
        }
    }
}
