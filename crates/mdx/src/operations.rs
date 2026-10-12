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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisasmArgs {
    pub rom: String,
    /// Even 24-bit CPU bus address, expressed as an integer.
    #[schemars(range(max = 16777215))]
    pub from: u32,
    #[serde(default = "count")]
    #[schemars(range(max = 4096))]
    pub count: usize,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BootArgs {
    pub rom: String,
    #[serde(default = "steps")]
    #[schemars(range(max = 100000000))]
    pub instructions: u64,
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
    pub input: Option<PathBuf>,
    pub out: Option<PathBuf>,
    /// Optional wall-clock bound; reached targets remain distinct from timeout.
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RamArgs {
    pub rom: String,
    #[serde(default = "steps")]
    #[schemars(range(max = 100000000))]
    pub instructions: u64,
    pub stop_at: Option<u32>,
    pub find: Option<u32>,
    /// [CPU address, byte count], bounded to one WRAM aperture.
    pub range: Option<(u32, u32)>,
    #[schemars(range(max = 3600000))]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotArgs {
    pub snapshot: String,
    pub out: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConformanceArgs {
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

fn descriptor<I: JsonSchema, O: JsonSchema>(name: &str, description: &str) -> Descriptor {
    let mut input_schema = serde_json::to_value(schemars::schema_for!(I)).unwrap();
    let mut output_schema = serde_json::to_value(
        schemars::generate::SchemaSettings::draft2020_12()
            .for_serialize()
            .into_generator()
            .into_root_schema_for::<Envelope<Option<Output<O>>>>(),
    )
    .unwrap();
    portable_schema(&mut input_schema);
    portable_schema(&mut output_schema);
    Descriptor {
        name: name.into(),
        mcp_name: name.replace('.', "_"),
        description: description.into(),
        input_schema,
        output_schema,
    }
}

pub fn descriptors() -> Vec<Descriptor> {
    vec![
        descriptor::<ImportArgs, ImportData>("artifact.import", "Import a local regular file into the immutable content-addressed store; return its reusable sha256 ID."),
        descriptor::<ReadArtifactArgs, ArtifactPage>("artifact.read", "Read a bounded byte page from a digest-verified artifact as hex or exact UTF-8; reports the next offset."),
        descriptor::<ViewArgs, ArtifactView>("artifact.view", "Inspect a JSON Pointer or page structured JSON/JSONL records from a verified artifact; useful for large boot responses and traces."),
        descriptor::<commands::memory::ReadArgs, commands::memory::ReadData>("memory.read", "Read a bounded page from verified captured memory without executing CPUs; WRAM aliases fold consistently."),
        descriptor::<commands::memory::SearchArgs, commands::memory::SearchData>("memory.search", "Search exact bytes across a complete captured buffer with explicit match pagination."),
        descriptor::<commands::memory::DiffArgs, commands::memory::DiffData>("memory.diff", "Compare two captured buffers completely and page changed ranges without inferring execution or causality."),
        descriptor::<SurveyArgs, SurveyData>("survey", "Batch up to 32 explicitly selected operations with per-task results and pending IDs; nested surveys are rejected."),
        descriptor::<commands::probe::ProbeArgs, commands::probe::ProbeData>("trace", "Trace bounded executed instructions with pre-instruction registers, optional frame/PC start and controller replay; records go to a JSONL artifact."),
        descriptor::<commands::probe::ProbeArgs, commands::probe::ProbeData>("watch", "Observe folded WRAM writes across aliases and byte/word/long widths, including same-value writes, with exact writer PC and pre-step registers."),
        descriptor::<HeaderArgs, commands::header::HeaderData>("header", "Inspect a Mega Drive cartridge header, vectors and declared memory ranges."),
        descriptor::<DisasmArgs, commands::disasm::DisasmData>("disasm", "Decode bounded cartridge instructions using mdcore's decoder; does not execute the game."),
        descriptor::<BootArgs, commands::boot::BootData>("boot", "Run a fresh bounded machine with optional controller replay; capture verified memory buffers and an image."),
        descriptor::<RamArgs, commands::ram::RamData>("ram.dump", "Run a bounded fresh machine, then inspect folded 64 KiB work RAM."),
        descriptor::<ConformanceArgs, commands::cpu::ConformanceData>("cpu.conformance", "Run canonical SingleStepTests fixtures with explicit coverage and timing limitations."),
        descriptor::<SnapshotArgs, commands::render::RenderData>("render", "Render digest-verified VDP snapshot buffers without executing CPUs."),
        descriptor::<SnapshotArgs, commands::graphics::GraphicsData>("graphics", "Export all mode-5 patterns, palettes, name tables and sprites from verified snapshot buffers."),
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

pub fn execute(name: &str, arguments: Value, context: &Context) -> Envelope<Value> {
    if serde_json::to_vec(&arguments).unwrap().len() > 1024 * 1024 {
        return Envelope::failure(
            &format!("mdx.{name}"),
            crate::artifacts::failure("limit_exceeded", "operation arguments exceed 1 MiB"),
        );
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute_inner(name, arguments, context)
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
        Ok(mut response) => {
            response.tool = format!("mdx.{name}");
            match finalize(name, &mut response, context) {
                Ok(()) => response,
                Err(failure) => Envelope::failure(&format!("mdx.{name}"), failure),
            }
        }
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
        "memory.diff" => references.extend([&data["left"], &data["right"]]),
        "trace" | "watch" => references.push(&data["records_artifact"]),
        "boot" | "render" | "graphics" => {
            if let Some(map) = data["memory_artifacts"].as_object() {
                references.extend(map.values());
            }
            if name == "boot" {
                references.push(&data["display"]["screenshot_artifact"]);
            }
            if name == "render" {
                references.push(&data["screenshot_artifact"]);
            }
            if name == "graphics" {
                references.extend([&data["tile_indices"], &data["layout"]]);
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
            erase(
                commands::disasm::run(
                    &crate::artifacts::resolve(&context.store, &args.rom)?,
                    args.from,
                    args.count,
                )?
                .0,
            )
        }
        "boot" => {
            let args: BootArgs = parse(arguments)?;
            let context = context.bounded(args.timeout_ms)?;
            erase(
                commands::boot::run_controlled(
                    &crate::artifacts::resolve(&context.store, &args.rom)?,
                    args.instructions,
                    args.trace,
                    args.out.as_deref(),
                    args.trace_registers,
                    &args.force_branch,
                    (!args.stop_at.is_empty()).then_some(args.stop_at),
                    args.frames,
                    args.input.as_deref(),
                    &context,
                )?
                .0,
            )
        }
        "ram.dump" => {
            let args: RamArgs = parse(arguments)?;
            erase(
                commands::ram::run_controlled(
                    &crate::artifacts::resolve(&context.store, &args.rom)?,
                    args.instructions,
                    args.stop_at,
                    args.find,
                    args.range,
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
        "render" | "graphics" => {
            let args: SnapshotArgs = parse(arguments)?;
            let path = crate::artifacts::resolve(&context.store, &args.snapshot)?;
            if name == "render" {
                erase(commands::render::run(
                    &path,
                    args.out.as_deref(),
                    &context.store,
                )?)
            } else {
                erase(commands::graphics::run(
                    &path,
                    args.out.as_deref(),
                    &context.store,
                )?)
            }
        }
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
