//! One typed operation registry for CLI schemas, CLI calls and MCP tools.
use crate::{
    commands,
    context::Context,
    envelope::{Coverage, Envelope, Failure},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Serialize, JsonSchema)]
pub struct ExternalResult {
    pub result_artifact: crate::artifacts::Artifact,
    pub retrieval_operation: String,
    pub encoding: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Output<T> {
    Inline(T),
    External(ExternalResult),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportArgs {
    pub path: String,
}

fn page_size() -> usize {
    4096
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Hex,
    Utf8,
}
impl Default for Encoding {
    fn default() -> Self {
        Self::Hex
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadArtifactArgs {
    pub source: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_size")]
    #[schemars(range(min = 1, max = 16384))]
    pub length: usize,
    #[serde(default)]
    pub encoding: Encoding,
}

#[derive(Serialize, JsonSchema)]
pub struct ImportData {
    pub artifact: crate::artifacts::Artifact,
}

#[derive(Serialize, JsonSchema)]
pub struct ArtifactPage {
    pub artifact: crate::artifacts::Artifact,
    pub offset: usize,
    pub length: usize,
    pub next_offset: Option<usize>,
    pub encoding: Encoding,
    pub content: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JsonFormat {
    Json,
    Jsonl,
}
impl Default for JsonFormat {
    fn default() -> Self {
        Self::Json
    }
}
fn view_limit() -> usize {
    16
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewArgs {
    pub source: String,
    /// RFC 6901 JSON Pointer. Empty selects the root; JSONL applies it to each record.
    #[serde(default)]
    pub pointer: String,
    #[serde(default)]
    pub format: JsonFormat,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "view_limit")]
    #[schemars(range(min = 1, max = 256))]
    pub limit: usize,
}
#[derive(Serialize, JsonSchema)]
pub struct ArtifactView {
    pub artifact: crate::artifacts::Artifact,
    pub pointer: String,
    pub format: JsonFormat,
    pub total_items: Option<usize>,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub value: Value,
}

fn steps() -> u64 {
    200_000
}
fn trace() -> usize {
    24
}
fn count() -> usize {
    64
}
fn failures() -> usize {
    20
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeaderArgs {
    pub rom: String,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DisasmCpu {
    #[default]
    M68000,
    Z80,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisasmArgs {
    pub rom: String,
    /// m68000: even CPU bus address in cartridge window 0x000000..0x3FFFFF,
    /// normally equal to ROM offset. z80: ROM file offset, or a Z80 RAM
    /// address 0x0000..0x1FFF with from_state.
    #[schemars(range(max = 16777215))]
    pub from: u32,
    /// "m68000" (default) or "z80".
    #[serde(default)]
    pub cpu: DisasmCpu,
    /// z80 from the ROM: the Z80 address the byte at `from` occupies once
    /// uploaded (default 0, the usual sound-driver load address).
    #[serde(default)]
    pub origin: u16,
    /// z80 only: disassemble this machine state's Z80 RAM instead of the ROM.
    #[serde(default)]
    #[schemars(with = "String")]
    pub from_state: Option<String>,
    #[serde(default = "count")]
    #[schemars(range(max = 4096))]
    pub count: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BootProfile {
    #[default]
    Full,
    Summary,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BootArgs {
    /// Resumable machine state artifact ID or JSON path. ROM must match exactly.
    #[serde(default)]
    #[schemars(with = "String")]
    pub from_state: Option<String>,
    /// Publish a complete modeled machine state for later continuation.
    #[serde(default)]
    pub save_state: bool,
    pub rom: String,
    /// Hard system-step budget, including interrupt/idle steps; default200000.
    /// This cap can stop a run BEFORE its frame target. Increase it when asking
    /// for later frames (e.g. frames120 with instructions5000000).
    #[serde(default = "steps")]
    #[schemars(range(max = 100000000))]
    pub instructions: u64,
    /// Completed video frames to run from reset or the loaded state, as a JSON integer.
    /// Not a duration or a count of serviced VINT interrupts. The run stops at
    /// this target OR another limit first; inspect reached_frames,
    /// frame_target_reached and termination. Instructions remains a hard cap.
    #[schemars(range(max = 100000))]
    pub frames: Option<u64>,
    #[serde(default = "trace")]
    #[schemars(range(max = 4096))]
    pub trace: usize,
    #[serde(default)]
    pub trace_registers: bool,
    #[serde(default)]
    pub force_branch: Vec<String>,
    #[serde(default)]
    #[schemars(length(max = 64))]
    pub stop_at: Vec<u32>,
    /// Controller replay JSON path; schema discovery includes file_schemas.controller_replay.
    pub input: Option<PathBuf>,
    /// Diagnostic RAM writes at frame boundaries (merged with the replay's pokes).
    #[serde(default)]
    #[schemars(length(max = 1000))]
    pub pokes: Vec<commands::input::Poke>,
    /// Work RAM values to record at every frame boundary into a CSV artifact
    /// (data.samples), for trajectory tests of a native port; needs frames.
    /// Row N is memory as run frame N begins, before its input; row 0 is the
    /// starting state.
    #[serde(default)]
    #[schemars(length(max = 64))]
    pub samples: Vec<commands::samples::SampleSpec>,
    /// Log 68000 writes into Z80 RAM, the bank register, BUSREQ and RESET to a
    /// JSONL artifact with a per-address summary; `{}` uses the defaults.
    #[serde(default)]
    pub sound_commands: Option<commands::sound_log::SoundCommandOptions>,
    /// Backup RAM image to load before running (<= 64 KiB from offset 0, e.g.
    /// a GPGX .srm). The cartridge must have backup RAM.
    #[serde(default)]
    #[schemars(with = "String")]
    pub sram: Option<PathBuf>,
    /// "full" (default) or "summary": frame/stop status, state and snapshot
    /// IDs, memory artifacts, screenshot and exceptions only, without traces,
    /// hot spots, registers, VDP/sound diagnostics or observation tables.
    #[serde(default)]
    pub profile: BootProfile,
    /// Console region at power-on: auto (header, like the reference), usa,
    /// japan or europe (PAL timing).
    #[serde(default)]
    pub region: commands::state::RegionChoice,
    /// Output DIRECTORY for this run's files (screenshot, memory dumps); not a
    /// report file. The JSON envelope is the operation's return value.
    pub out: Option<PathBuf>,
    /// Optional wall-clock bound; reached targets remain distinct from timeout.
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RamArgs {
    /// Resumable machine state artifact ID or JSON path. ROM must match exactly.
    #[serde(default)]
    #[schemars(with = "String")]
    pub from_state: Option<String>,
    /// Publish a complete modeled machine state for later continuation.
    #[serde(default)]
    pub save_state: bool,
    /// Console region at power-on: auto (header, like the reference), usa,
    /// japan or europe (PAL timing).
    #[serde(default)]
    pub region: commands::state::RegionChoice,
    pub rom: String,
    #[serde(default = "steps")]
    #[schemars(range(max = 100000000))]
    pub instructions: u64,
    pub stop_at: Option<u32>,
    pub find: Option<u32>,
    /// [folded WRAM offset or CPU WRAM alias, byte count]; integer or decimal/0x-prefixed text.
    /// At most 64 KiB, wrapping within WRAM. Non-WRAM full addresses are rejected.
    pub range: Option<(commands::ram::RangeValue, commands::ram::RangeValue)>,
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GraphicsArgs {
    pub snapshot: String,
    /// Output DIRECTORY for exported images; not a file name.
    pub out: Option<PathBuf>,
    /// Compose caller-selected hardware sprites into one transparent PNG each.
    #[serde(default)]
    #[schemars(length(max = 16))]
    pub sprite_groups: Vec<commands::graphics::SpriteGroup>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotArgs {
    pub snapshot: String,
    /// Output DIRECTORY for exported images; not a file name.
    pub out: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConformanceArgs {
    /// SingleStepTests 68000 directory containing v1/*.json.gz, or a bundled regression JSON file.
    pub suite: PathBuf,
    pub filter: Option<String>,
    #[serde(default = "failures")]
    #[schemars(range(max = 4096))]
    pub failures: usize,
    #[serde(default)]
    pub relocate: bool,
    #[serde(default)]
    pub compare_cycles: bool,
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

fn schema_limit() -> usize {
    4
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaArgs {
    pub command: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "schema_limit")]
    #[schemars(range(min = 1, max = 4))]
    pub limit: usize,
}

#[derive(Serialize, JsonSchema)]
pub struct Descriptor {
    pub name: String,
    pub mcp_name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    /// Schemas for local file inputs used by this operation.
    pub file_schemas: std::collections::BTreeMap<String, Value>,
}

#[derive(Serialize, JsonSchema)]
pub struct SchemaData {
    pub operations: Vec<Descriptor>,
    pub available_operations: Vec<String>,
    pub total: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurveyItem {
    pub id: String,
    pub operation: String,
    pub arguments: Value,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurveyArgs {
    #[schemars(length(min = 1, max = 32))]
    pub tasks: Vec<SurveyItem>,
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct SurveyResult {
    pub id: String,
    pub response: Envelope<Value>,
}

#[derive(Serialize, JsonSchema)]
pub struct SurveyData {
    pub results: Vec<SurveyResult>,
    pub pending_ids: Vec<String>,
}

// Replace Rust-specific numeric formats with portable mathematical bounds,
// retaining any narrower operation limits already declared in the schema.
fn portable_schema(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let bounds = match object.get("format").and_then(Value::as_str) {
                Some("uint") => Some((serde_json::json!(0), serde_json::json!(usize::MAX))),
                Some("uint8") => Some((serde_json::json!(0), serde_json::json!(u8::MAX))),
                Some("uint16") => Some((serde_json::json!(0), serde_json::json!(u16::MAX))),
                Some("uint32") => Some((serde_json::json!(0), serde_json::json!(u32::MAX))),
                Some("uint64") => Some((serde_json::json!(0), serde_json::json!(u64::MAX))),
                Some("int") => Some((serde_json::json!(isize::MIN), serde_json::json!(isize::MAX))),
                Some("int8") => Some((serde_json::json!(i8::MIN), serde_json::json!(i8::MAX))),
                Some("int16") => Some((serde_json::json!(i16::MIN), serde_json::json!(i16::MAX))),
                Some("int32") => Some((serde_json::json!(i32::MIN), serde_json::json!(i32::MAX))),
                Some("int64") => Some((serde_json::json!(i64::MIN), serde_json::json!(i64::MAX))),
                Some("float" | "double") => {
                    object.remove("format");
                    None
                }
                _ => None,
            };
            if let Some((minimum, maximum)) = bounds {
                object.remove("format");
                if object
                    .get("minimum")
                    .and_then(Value::as_f64)
                    .is_none_or(|v| v < minimum.as_f64().unwrap())
                {
                    object.insert("minimum".into(), minimum);
                }
                if object
                    .get("maximum")
                    .and_then(Value::as_f64)
                    .is_none_or(|v| v > maximum.as_f64().unwrap())
                {
                    object.insert("maximum".into(), maximum);
                }
            }
            for child in object.values_mut() {
                portable_schema(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                portable_schema(child);
            }
        }
        _ => {}
    }
}

/// Common, request-local paths exposed by every CLI/API/MCP operation.
#[derive(Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceArgs {
    /// Absolute existing directory of the calling agent's project. Relative
    /// input/output paths resolve here, and artifacts default to .mdx/artifacts.
    /// Pass the path as a quoted JSON string; omit it to retain the launch context.
    #[schemars(with = "String")]
    pub project_root: Option<PathBuf>,
    /// Override the artifact store for this call (absolute or relative to
    /// project_root). Use the same scope when retrieving sha256 IDs. Agents
    /// may select separate stores, or share the concurrent-safe project store.
    #[schemars(with = "String")]
    pub artifact_store: Option<PathBuf>,
}

fn descriptor<I: JsonSchema, O: JsonSchema>(name: &str, description: &str) -> Descriptor {
    let mut input_schema = serde_json::to_value(schemars::schema_for!(I)).unwrap();
    let workspace_schema = serde_json::to_value(schemars::schema_for!(WorkspaceArgs)).unwrap();
    let properties = input_schema["properties"].as_object_mut().unwrap();
    for (name, schema) in workspace_schema["properties"].as_object().unwrap() {
        assert!(
            !properties.contains_key(name),
            "operation shadows a workspace argument"
        );
        properties.insert(name.clone(), schema.clone());
    }
    let mut output_schema = serde_json::to_value(
        schemars::generate::SchemaSettings::draft2020_12()
            .for_serialize()
            .into_generator()
            .into_root_schema_for::<Envelope<Option<Output<O>>>>(),
    )
    .unwrap();
    portable_schema(&mut input_schema);
    portable_schema(&mut output_schema);
    let mut file_schemas = std::collections::BTreeMap::new();
    if matches!(name, "boot" | "trace" | "watch" | "audio") {
        let mut replay =
            serde_json::to_value(schemars::schema_for!(commands::input::Replay)).unwrap();
        portable_schema(&mut replay);
        file_schemas.insert("controller_replay".into(), replay);
    }
    Descriptor {
        name: name.into(),
        mcp_name: name.replace('.', "_"),
        description: description.into(),
        input_schema,
        output_schema,
        file_schemas,
    }
}

pub fn descriptors() -> Vec<Descriptor> {
    vec![
        descriptor::<commands::template::CreateArgs, commands::template::CreateData>("template.create", "Create a Rust/Bevy port workspace (md-bevy template crate, engine with sandbox, game, importers, docs) at an absolute destination in the calling agent's workspace, e.g. the ROM's folder; existing files are kept, colliding paths refused. Read guide topic port first."),
        descriptor::<commands::template::UpdateArgs, commands::template::UpdateData>("template.update", "Bring a template.create project up to date: replaces unedited md-bevy files, writes <file>.mdx-new beside edited ones, copies changed seeded files under .mdx/template-update. dry_run reports only."),
        descriptor::<commands::guide::GuideArgs, commands::guide::GuideData>("guide", "Built-in workflow guidance by topic (overview, port, ghidra). Read port before starting a native port; ghidra before ghidra_* tools."),
        descriptor::<ImportArgs, ImportData>("artifact.import", "Import a local regular file into the immutable content-addressed store; return its reusable sha256 ID."),
        descriptor::<ReadArtifactArgs, ArtifactPage>("artifact.read", "Read a bounded byte page from a digest-verified artifact as hex or exact UTF-8; reports the next offset."),
        descriptor::<ViewArgs, ArtifactView>("artifact.view", "Inspect a JSON Pointer or page structured JSON/JSONL records from a verified artifact; useful for large boot responses and traces."),
        descriptor::<commands::memory::ReadArgs, commands::memory::ReadData>("memory.read", "Read a bounded page from verified captured memory without executing CPUs; WRAM aliases fold consistently."),
        descriptor::<commands::memory::SearchArgs, commands::memory::SearchData>("memory.search", "Search exact bytes across a complete captured buffer with explicit match pagination."),
        descriptor::<commands::image::CompareArgs, commands::image::CompareData>("image.compare", "Compare two PNGs pixel by pixel (e.g. mdx screenshot vs native port): absolute changed counts, named regions, change bounds and a diff PNG."),
        descriptor::<commands::memory::DiffArgs, commands::memory::DiffData>("memory.diff", "Compare two captured buffers completely and page changed ranges without inferring execution or causality."),
        descriptor::<SurveyArgs, SurveyData>("survey", "Batch up to 32 explicitly selected operations with per-task results and pending IDs; nested surveys are rejected."),
        descriptor::<commands::probe::ProbeArgs, commands::probe::ProbeData>("trace", "Trace bounded executed instructions from reset or from_state; frames, start_frame and replay frames are relative to the starting state. save_state publishes a resumable artifact; records go to JSONL. coverage:true also records every distinct PC and control transfer of the whole run (ghidra evidence)."),
        descriptor::<commands::probe::ProbeArgs, commands::probe::ProbeData>("watch", "Observe folded WRAM writes with exact writer PC and pre-step registers. Supports from_state/save_state; frames, start_frame and controller replay are relative to the starting state."),
        descriptor::<HeaderArgs, commands::header::HeaderData>("header", "Inspect a Mega Drive cartridge header, vectors and declared memory ranges."),
        descriptor::<DisasmArgs, commands::disasm::DisasmData>("disasm", "Decode bounded cartridge instructions using mdcore's decoder; does not execute the game."),
        descriptor::<commands::audio::AudioArgs, commands::audio::AudioData>("audio", "Synthesize bounded stereo FM/PSG WAV audio. frames is capture duration after start_frame warmup; both are relative to reset/from_state. Supports replay/save_state, project-local artifacts and optional output directory. Inspect frame_target_reached, termination and sample indexes."),
        descriptor::<BootArgs, commands::boot::BootData>("boot", "Run from reset or from_state to a completed-video-frame target OR its system-step budget. Frames and controller replay are relative to the starting state; reached_frames stays absolute. The default200000-step cap can stop early. save_state publishes a resumable state_artifact; snapshot_artifact captures buffers/image."),
        descriptor::<RamArgs, commands::ram::RamData>("ram.dump", "Run from reset or from_state for a bounded number of new steps, then inspect folded 64 KiB work RAM. instructions0 reads the loaded state without advancing; save_state publishes a resumable artifact."),
        descriptor::<ConformanceArgs, commands::cpu::ConformanceData>("cpu.conformance", "Run canonical SingleStepTests fixtures with explicit coverage and timing limitations."),
        descriptor::<SnapshotArgs, commands::render::RenderData>("render", "Render digest-verified VDP snapshot buffers without executing CPUs."),
        descriptor::<GraphicsArgs, commands::graphics::GraphicsData>("graphics", "Export all mode-5 patterns, palettes, name tables and sprites from verified snapshot buffers; sprite_groups composes caller-selected sprites into transparent images."),
        descriptor::<commands::ghidra::DoctorArgs, Value>("ghidra.doctor", "Check that the optional Ghidra install and Java are discoverable (MDX_GHIDRA_HOME or ghidra_home). Usually unnecessary: ghidra queries import automatically."),
        descriptor::<commands::ghidra::AnalyzeArgs, Value>("ghidra.analyze", "Optional: prewarm a cached Ghidra analysis of a ROM (m68000, or cpu z80 sound driver) with vectors, header, hardware blocks, bank mapping, from_state RAM and runtime evidence. Returns the analysis handle. See guide topic ghidra."),
        descriptor::<commands::ghidra::FunctionsArgs, Value>("ghidra.functions", "List Ghidra's functions (entries, names, signatures, body ranges) for a rom or analysis handle; filter by name, page with offset/limit."),
        descriptor::<commands::ghidra::DecompileArgs, Value>("ghidra.decompile", "Pseudocode, instruction listing and callers/callees for the function containing addr (hex, e.g. a watch writer PC). Imports the rom automatically (about a minute first time). Pseudocode is a hypothesis for hand-written 68000; needs local Ghidra."),
        descriptor::<commands::ghidra::DecompileBatchArgs, Value>("ghidra.decompile_batch", "Decompile up to 200 functions (addrs, hex) in one Ghidra run and return them as one result; brief by default (pseudocode and named callers/callees, no listings). Use instead of repeated ghidra_decompile calls for sweeps."),
        descriptor::<commands::ghidra::XrefsArgs, Value>("ghidra.xrefs", "References to and from addr (code, ROM data, work RAM alias or hardware register) in a Ghidra analysis, with type and containing function; paged. Indirect references need runtime evidence."),
        descriptor::<commands::ghidra::AnnotateArgs, Value>("ghidra.annotate", "Apply function renames and comments (functions:[{addr,name,comment}]) and data names (labels:[{addr,name,size,comment}], e.g. RAM globals) inline or from a file to a Ghidra analysis as a new revision. User-name collisions fail; generated names yield."),
        descriptor::<commands::ghidra::ExportArgs, Value>("ghidra.export_symbols", "Export a Ghidra analysis's function names as .symbols.json bound to each body's bytes; reusable as symbols seeds for later analyses."),
        descriptor::<SchemaArgs, SchemaData>("schema", "Describe operation input/output JSON Schemas without loading a cartridge or producing analysis artifacts."),
    ]
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Failure> {
    serde_json::from_value(value).map_err(|e| Failure {
        code: "invalid_arguments".into(),
        message: e.to_string(),
        remediation: None,
    })
}

fn erase<T: Serialize>(envelope: Envelope<T>) -> Envelope<Value> {
    serde_json::from_value(serde_json::to_value(envelope).expect("serializable operation"))
        .expect("valid envelope")
}

fn workspace_context(
    name: &str,
    arguments: &mut Value,
    parent: &Context,
) -> Result<Context, Failure> {
    let object = arguments.as_object_mut().ok_or_else(|| {
        crate::artifacts::failure("invalid_arguments", "operation arguments must be an object")
    })?;
    let mut workspace = serde_json::Map::new();
    for key in ["project_root", "artifact_store"] {
        if let Some(value) = object.remove(key) {
            workspace.insert(key.into(), value);
        }
    }
    let workspace: WorkspaceArgs = parse(Value::Object(workspace))?;
    let mut context = parent.clone();
    if let Some(root) = workspace.project_root {
        if !root.is_absolute() || !root.is_dir() {
            return Err(crate::artifacts::failure(
                "invalid_arguments",
                "project_root must be an absolute existing directory",
            ));
        }
        context.store = root.join(".mdx/artifacts");
        context.root = root;
    }
    if !context.root.is_absolute() {
        context.root = crate::artifacts::absolute(&context.root)?;
    }
    let rooted = |path: PathBuf| {
        if path.is_absolute() {
            path
        } else {
            context.root.join(path)
        }
    };
    context.store = rooted(workspace.artifact_store.unwrap_or(context.store));
    // Resolve only path fields declared by the operation contract. Never
    // interpret arbitrary JSON data, JSON Pointers, or survey child arguments.
    let paths: &[&str] = match name {
        "header" => &["rom"],
        "disasm" => &["rom", "from_state"],
        "ram.dump" => &["rom", "from_state"],
        "boot" => &["rom", "input", "out", "from_state", "sram"],
        "audio" => &["rom", "input", "out", "from_state"],
        "trace" | "watch" => &["rom", "input", "from_state"],
        "render" | "graphics" => &["snapshot", "out"],
        "memory.read" | "memory.search" => &["snapshot"],
        "memory.diff" | "image.compare" => &["left", "right", "out"],
        "artifact.import" => &["path"],
        "artifact.read" | "artifact.view" => &["source"],
        "cpu.conformance" => &["suite"],
        name if name.starts_with("ghidra.") => &["rom", "from_state", "analysis", "out", "file", "ghidra_home"],
        // Template destinations retain the explicit absolute-path contract.
        _ => &[],
    };
    // Repeatable path arguments; IDs stay IDs.
    if name.starts_with("ghidra.") {
        for key in ["evidence", "symbols"] {
            if let Some(Value::Array(items)) = object.get_mut(key) {
                for item in items {
                    if let Some(source) = item.as_str().filter(|s| !s.starts_with("sha256:")) {
                        *item = serde_json::to_value(rooted(PathBuf::from(source))).unwrap();
                    }
                }
            }
        }
    }
    for key in paths {
        if let Some(value) = object.get_mut(*key) {
            if let Some(source) = value.as_str() {
                let accepts_id = matches!(
                    *key,
                    "rom" | "snapshot" | "source" | "path" | "left" | "right" | "from_state"
                );
                if !accepts_id || !source.starts_with("sha256:") {
                    *value = serde_json::to_value(rooted(PathBuf::from(source))).unwrap();
                }
            }
        }
    }
    Ok(context)
}

pub fn execute(name: &str, mut arguments: Value, context: &Context) -> Envelope<Value> {
    if serde_json::to_vec(&arguments).unwrap().len() > 1024 * 1024 {
        return Envelope::failure(
            &format!("mdx.{name}"),
            crate::artifacts::failure("limit_exceeded", "operation arguments exceed 1 MiB"),
        );
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let context = workspace_context(name, &mut arguments, context)?;
        let mut response = execute_inner(name, arguments, &context)?;
        response.tool = format!("mdx.{name}");
        finalize(name, &mut response, &context)?;
        Ok(response)
    }))
    .unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown core panic");
        Err(crate::artifacts::failure(
            "analysis_failed",
            format!("analysis could not complete: {message}"),
        ))
    });
    match result {
        Ok(response) => response,
        Err(failure) => {
            let mut response = Envelope::failure(&format!("mdx.{name}"), failure);
            if matches!(
                response.error.as_ref().unwrap().code.as_str(),
                "invalid_arguments" | "usage" | "limit_exceeded"
            ) {
                response.exit = crate::envelope::Exit::Usage;
            }
            response
        }
    }
}

fn store_references(name: &str, data: &Value, context: &Context) -> Result<(), Failure> {
    // Only fields owned by operation contracts are references. Arbitrary JSON
    // returned by artifact.view is data, never an implicit filesystem request.
    let mut references = Vec::new();
    match name {
        "artifact.import" | "artifact.read" | "artifact.view" | "memory.read" | "memory.search" => {
            references.push(&data["artifact"])
        }
        "audio" => references.push(&data["wav_artifact"]),
        "memory.diff" => references.extend([&data["left"], &data["right"]]),
        "image.compare" => references.push(&data["diff_artifact"]),
        "trace" | "watch" => {
            references.push(&data["records_artifact"]);
            if data["coverage"].is_object() {
                references.push(&data["coverage"]["coverage_artifact"]);
            }
        }
        "boot" | "render" | "graphics" => {
            if let Some(map) = data["memory_artifacts"].as_object() {
                references.extend(map.values());
            }
            if name == "boot" {
                references.push(&data["display"]["screenshot_artifact"]);
                if data["samples"].is_object() {
                    references.push(&data["samples"]["artifact"]);
                }
            }
            if name == "render" {
                references.push(&data["screenshot_artifact"]);
            }
            if name == "graphics" {
                references.extend([&data["tile_indices"], &data["layout"]]);
                if let Some(groups) = data["sprite_groups"].as_array() {
                    references.extend(groups.iter().map(|g| &g["image"]));
                }
                if let Some(sheets) = data["tile_sheets"].as_array() {
                    references.extend(sheets);
                }
            }
        }
        _ => {}
    }
    for value in references {
        let artifact: crate::artifacts::Artifact = serde_json::from_value(value.clone())
            .map_err(|e| crate::artifacts::failure("artifact_invalid", e.to_string()))?;
        let bytes = crate::artifacts::verified(&artifact)?;
        crate::artifacts::put(&context.store, &bytes)?;
    }
    Ok(())
}

fn finalize(name: &str, response: &mut Envelope<Value>, context: &Context) -> Result<(), Failure> {
    if name == "schema" {
        return Ok(());
    }
    store_references(name, &response.data, context)?;
    if name == "boot" {
        let artifact =
            crate::artifacts::put(&context.store, &serde_json::to_vec(response).unwrap())?;
        response.data["snapshot_artifact"] = serde_json::to_value(artifact).unwrap();
    }
    let bytes = serde_json::to_vec(response).expect("serializable response");
    if bytes.len() > 64 * 1024 {
        let artifact = crate::artifacts::put(&context.store, &bytes)?;
        response.data = serde_json::to_value(ExternalResult {
            result_artifact: artifact,
            retrieval_operation: "artifact.read".into(),
            encoding: "json".into(),
        })
        .unwrap();
        response.coverage.limits.push("inline response <= 64 KiB; complete response stored as a digest-verified JSON artifact".into());
    }
    Ok(())
}

fn execute_inner(
    name: &str,
    arguments: Value,
    context: &Context,
) -> Result<Envelope<Value>, Failure> {
    context.check()?;
    Ok(match name {
        "template.create" => erase(commands::template::create(parse(arguments)?, context)?),
        "template.update" => erase(commands::template::update(parse(arguments)?, context)?),
        "guide" => erase(commands::guide::run(parse(arguments)?)?),
        "artifact.import" => {
            let args: ImportArgs = parse(arguments)?;
            let path = crate::artifacts::resolve(&context.store, &args.path)?;
            let artifact = crate::artifacts::put(
                &context.store,
                &crate::artifacts::read_bounded(&path, 64 * 1024 * 1024)?,
            )?;
            erase(Envelope::ok(
                "mdx.artifact.import",
                None,
                Coverage::complete(vec![
                    "exact file bytes committed by SHA-256; no interpretation executed".into(),
                ]),
                ImportData { artifact },
            ))
        }
        "artifact.read" => {
            let args: ReadArtifactArgs = parse(arguments)?;
            if args.length == 0 || args.length > 16384 {
                return Err(crate::artifacts::failure(
                    "limit_exceeded",
                    "artifact pages must be 1..16384 bytes",
                ));
            }
            let path = crate::artifacts::resolve(&context.store, &args.source)?;
            let bytes = crate::artifacts::read_bounded(&path, 64 * 1024 * 1024)?;
            if args.offset > bytes.len() {
                return Err(crate::artifacts::failure(
                    "invalid_arguments",
                    "offset is beyond artifact end",
                ));
            }
            let end = (args.offset + args.length).min(bytes.len());
            let selected = &bytes[args.offset..end];
            let content = match args.encoding {
                Encoding::Hex => selected.iter().map(|b| format!("{b:02x}")).collect(),
                Encoding::Utf8 => std::str::from_utf8(selected)
                    .map_err(|e| crate::artifacts::failure("invalid_utf8_page", e.to_string()))?
                    .to_owned(),
            };
            let sha256 = crate::artifacts::digest(&bytes);
            let artifact = crate::artifacts::Artifact {
                id: format!("sha256:{sha256}"),
                path,
                bytes: bytes.len(),
                sha256,
            };
            erase(Envelope::ok(
                "mdx.artifact.read",
                None,
                Coverage::complete(vec![format!(
                    "artifact byte range {}..{end} of {}",
                    args.offset,
                    bytes.len()
                )]),
                ArtifactPage {
                    artifact,
                    offset: args.offset,
                    length: selected.len(),
                    next_offset: (end < bytes.len()).then_some(end),
                    encoding: args.encoding,
                    content,
                },
            ))
        }
        "artifact.view" => {
            let args: ViewArgs = parse(arguments)?;
            if args.limit == 0
                || args.limit > 256
                || (!args.pointer.is_empty() && !args.pointer.starts_with('/'))
            {
                return Err(crate::artifacts::failure(
                    "invalid_arguments",
                    "view requires limit 1..256 and an empty or slash-prefixed JSON Pointer",
                ));
            }
            let path = crate::artifacts::resolve(&context.store, &args.source)?;
            let bytes = crate::artifacts::read_bounded(&path, 64 * 1024 * 1024)?;
            let sha256 = crate::artifacts::digest(&bytes);
            let artifact = crate::artifacts::Artifact {
                id: format!("sha256:{sha256}"),
                path,
                bytes: bytes.len(),
                sha256,
            };
            let parsed = match args.format {
                JsonFormat::Json => serde_json::from_slice::<Value>(&bytes).map_err(|e| {
                    crate::artifacts::failure("artifact_json_invalid", e.to_string())
                })?,
                JsonFormat::Jsonl => {
                    let text = std::str::from_utf8(&bytes).map_err(|e| {
                        crate::artifacts::failure("artifact_json_invalid", e.to_string())
                    })?;
                    let rows = text
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .map(|line| serde_json::from_str::<Value>(line))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| {
                            crate::artifacts::failure("artifact_json_invalid", e.to_string())
                        })?;
                    Value::Array(rows)
                }
            };
            let select = |value: &Value| {
                value.pointer(&args.pointer).cloned().ok_or_else(|| {
                    crate::artifacts::failure(
                        "json_pointer_not_found",
                        "selected JSON Pointer does not exist",
                    )
                })
            };
            let selected = if matches!(args.format, JsonFormat::Json) {
                select(&parsed)?
            } else {
                parsed
            };
            let (total_items, next_offset, value) = if let Some(array) = selected.as_array() {
                if args.offset > array.len() {
                    return Err(crate::artifacts::failure(
                        "invalid_arguments",
                        "view offset exceeds array length",
                    ));
                }
                let end = (args.offset + args.limit).min(array.len());
                let rows = if matches!(args.format, JsonFormat::Jsonl) {
                    array[args.offset..end]
                        .iter()
                        .map(select)
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    array[args.offset..end].to_vec()
                };
                (
                    Some(array.len()),
                    (end < array.len()).then_some(end),
                    Value::Array(rows),
                )
            } else {
                if args.offset != 0 {
                    return Err(crate::artifacts::failure(
                        "invalid_arguments",
                        "object/scalar views do not have an array offset",
                    ));
                }
                (None, None, selected)
            };
            erase(Envelope::ok(
                "mdx.artifact.view",
                None,
                Coverage::complete(vec![format!(
                    "digest-verified {:?} view at pointer {:?}; explicit array pagination",
                    args.format, args.pointer
                )]),
                ArtifactView {
                    artifact,
                    pointer: args.pointer,
                    format: args.format,
                    total_items,
                    offset: args.offset,
                    next_offset,
                    value,
                },
            ))
        }
        "memory.read" => erase(commands::memory::read(parse(arguments)?, context)?),
        "memory.search" => erase(commands::memory::search(parse(arguments)?, context)?),
        "memory.diff" => erase(commands::memory::diff(parse(arguments)?, context)?),
        "image.compare" => erase(commands::image::compare(parse(arguments)?, context)?),
        "trace" | "watch" => erase(commands::probe::run(parse(arguments)?, name, context)?),
        "survey" => {
            let args: SurveyArgs = parse(arguments)?;
            if args.tasks.is_empty() || args.tasks.len() > 32 {
                return Err(crate::artifacts::failure(
                    "limit_exceeded",
                    "survey requires 1..32 tasks",
                ));
            }
            let mut ids = std::collections::BTreeSet::new();
            for task in &args.tasks {
                if task.id.is_empty() || !ids.insert(&task.id) || task.operation == "survey" {
                    return Err(crate::artifacts::failure(
                        "invalid_arguments",
                        "survey requires unique nonempty task IDs and no nested surveys",
                    ));
                }
            }
            let context = context.bounded(args.timeout_ms)?;
            let total_tasks = args.tasks.len();
            drop(ids);
            let mut results = Vec::new();
            let mut pending_ids = Vec::new();
            for task in args.tasks {
                if context.stop_reason().is_some() {
                    pending_ids.push(task.id);
                    continue;
                }
                let mut child_context = context.clone();
                child_context.progress = None;
                let response = execute(&task.operation, task.arguments, &child_context);
                results.push(SurveyResult {
                    id: task.id,
                    response,
                });
                context.report(results.len() as u64, total_tasks as u64);
            }
            let mut coverage = Coverage::complete(vec![format!(
                "{} tasks completed; {} pending",
                results.len(),
                pending_ids.len()
            )]);
            if !pending_ids.is_empty() || results.iter().any(|r| !r.response.ok) {
                coverage.status = crate::envelope::CoverageStatus::Partial;
                coverage.limitations.push("batch contains unsuccessful/partial tasks or pending work; inspect each task response".into());
            }
            erase(Envelope::ok(
                "mdx.survey",
                None,
                coverage,
                SurveyData {
                    results,
                    pending_ids,
                },
            ))
        }
        "header" => {
            let args: HeaderArgs = parse(arguments)?;
            erase(commands::header::run(&crate::artifacts::resolve(&context.store, &args.rom)?)?.0)
        }
        "disasm" => {
            let args: DisasmArgs = parse(arguments)?;
            let rom = crate::artifacts::resolve(&context.store, &args.rom)?;
            match args.cpu {
                DisasmCpu::M68000 if args.from_state.is_some() || args.origin != 0 => {
                    return Err(crate::artifacts::failure(
                        "invalid_arguments",
                        "origin and from_state apply to cpu z80 only",
                    ));
                }
                DisasmCpu::M68000 => erase(commands::disasm::run(&rom, args.from, args.count)?.0),
                DisasmCpu::Z80 => {
                    let source = match args.from_state.as_deref() {
                        Some(state) => commands::disasm::Z80Source::State(state),
                        None => commands::disasm::Z80Source::Rom { origin: args.origin },
                    };
                    erase(commands::disasm::run_z80(&rom, args.from, args.count, source, context)?.0)
                }
            }
        }
        "audio" => erase(commands::audio::run(parse(arguments)?, context)?),
        "boot" => {
            let args: BootArgs = parse(arguments)?;
            let context = context.bounded(args.timeout_ms)?;
            erase(
                commands::boot::run_resumable(
                    &crate::artifacts::resolve(&context.store, &args.rom)?,
                    args.instructions,
                    args.trace,
                    args.out.as_deref(),
                    args.trace_registers,
                    &args.force_branch,
                    (!args.stop_at.is_empty()).then_some(args.stop_at),
                    args.frames,
                    args.input.as_deref(),
                    &args.pokes,
                    args.sound_commands.as_ref(),
                    args.sram.as_deref(),
                    args.region,
                    args.from_state.as_deref(),
                    args.save_state,
                    &args.samples,
                    &context,
                )
                .map(|(mut envelope, code)| {
                    if args.profile == BootProfile::Summary {
                        let data = &mut envelope.data;
                        data.observations.clear();
                        data.input_replay = None;
                        data.first_addresses.clear();
                        data.last_addresses.clear();
                        data.hottest_addresses.clear();
                        data.final_registers = None;
                        data.vdp = None;
                        data.unmapped_access_samples.clear();
                    }
                    (envelope, code)
                })?
                .0,
            )
        }
        "ram.dump" => {
            let args: RamArgs = parse(arguments)?;
            let range = args
                .range
                .as_ref()
                .map(|(addr, len)| Ok((addr.parse()?, len.parse()?)))
                .transpose()?;
            erase(
                commands::ram::run_resumable(
                    &crate::artifacts::resolve(&context.store, &args.rom)?,
                    args.instructions,
                    args.stop_at,
                    args.find,
                    range,
                    args.from_state.as_deref(),
                    args.save_state,
                    args.region,
                    &context.bounded(args.timeout_ms)?,
                )?
                .0,
            )
        }
        "cpu.conformance" => {
            let args: ConformanceArgs = parse(arguments)?;
            erase(commands::cpu::run_controlled(
                &args.suite,
                args.filter.as_deref(),
                args.failures,
                args.relocate,
                args.compare_cycles,
                &context.bounded(args.timeout_ms)?,
            )?)
        }
        "render" => {
            let args: SnapshotArgs = parse(arguments)?;
            let path = crate::artifacts::resolve(&context.store, &args.snapshot)?;
            erase(commands::render::run(&path, args.out.as_deref(), &context.store)?)
        }
        "graphics" => {
            let args: GraphicsArgs = parse(arguments)?;
            let path = crate::artifacts::resolve(&context.store, &args.snapshot)?;
            erase(commands::graphics::run(
                &path,
                args.out.as_deref(),
                &context.store,
                &args.sprite_groups,
            )?)
        }
        name if name.starts_with("ghidra.") => commands::ghidra::run(name, arguments, context)?,
        "schema" => {
            let args: SchemaArgs = parse(arguments)?;
            if args.limit == 0 || args.limit > 4 {
                return Err(crate::artifacts::failure(
                    "limit_exceeded",
                    "schema pages contain 1..4 operations",
                ));
            }
            let descriptors = descriptors();
            let available_operations = descriptors.iter().map(|d| d.name.clone()).collect();
            let selected: Vec<_> = descriptors
                .into_iter()
                .filter(|d| args.command.as_ref().is_none_or(|c| c == &d.name))
                .collect();
            let total = selected.len();
            if total == 0 || args.offset > total {
                return Err(Failure {
                    code: "usage".into(),
                    message: "unknown schema operation or page offset".into(),
                    remediation: None,
                });
            }
            let operations: Vec<_> = selected
                .into_iter()
                .skip(args.offset)
                .take(args.limit)
                .collect();
            let end = args.offset + operations.len();
            erase(Envelope::ok("mdx.schema", None, Coverage::complete(vec![format!("schemas {}..{end} of {total}; derived from typed inputs and serialized outputs; no analysis/filesystem writes",args.offset)]),
                SchemaData { operations,available_operations,total,offset:args.offset,next_offset:(end<total).then_some(end) }))
        }
        _ => {
            return Err(Failure {
                code: "usage".into(),
                message: format!("unknown operation {name:?}"),
                remediation: Some("use mdx schema to list operations".into()),
            })
        }
    })
}

pub fn invocation(name: &str, arguments: Value) -> Value {
    json!({"operation":name,"arguments":arguments})
}
