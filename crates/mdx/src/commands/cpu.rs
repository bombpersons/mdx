//! `mdx cpu conformance` -- run the SingleStepTests/680x0 suite against the core.
//!
//! The suite gives complete processor and memory state either side of a single
//! instruction, so a failure names the exact opcode that is wrong instead of
//! "the game crashed somewhere".
//!
//! Fixture PC identifies the opcode and the two prefetched words already
//! contain the opcode and its successor. Declared RAM supplies later fetches.
//! Canonical mode preserves these inputs and compares PC and prefetch as well
//! as registers, status, and declared memory. Cycle/bus timing is not compared.

use crate::envelope::{Coverage, Envelope, Failure, Subject, Warning};
use mdcore::cpu::bus::Bus as _;
use mdcore::cpu::m68000::M68000;
use mdcore::cpu::status::StatusRegister;
use mdcore::SparseBus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const TOOL: &str = "mdx.cpu.conformance";

const REGISTERS: [&str; 8] = ["d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7"];
const MAX_FILES: usize = 4096;
const MAX_CASES: usize = 20_000_000;

/// One failing case, with the specific mismatch that caused it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Failure_ {
    pub file: String,
    pub case: String,
    pub opcode: String,
    /// e.g. `d3`, `sr`, `ccr`, `usp`, `a7`, `ram[0x1234]`
    pub field: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct PerFile {
    pub file: String,
    pub cases: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ConformanceData {
    pub source_files: Vec<FixtureIdentity>,
    /// Declared upstream provenance for bundled subsets, not hashes verified against upstream files.
    pub bundled_sources: Vec<BundledSource>,
    pub termination: String,
    pub failure_details_omitted: usize,
    pub cycles_compared: bool,
    pub suite_path: String,
    pub files_run: usize,
    pub files_total: usize,
    pub cases_run: usize,
    pub cases_passed: usize,
    pub cases_failed: usize,
    /// Per-instruction breakdown, sorted worst-first so the response leads with
    /// what still needs work.
    pub per_instruction: Vec<PerFile>,
    pub failures: Vec<Failure_>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FixtureIdentity {
    pub file: String,
    pub file_sha256: String,
    pub compressed_bytes: usize,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundledSource {
    pub file: String,
    pub sha256: String,
    pub cases: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleEntry {
    file: String,
    case: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleInput {
    sources: Vec<BundledSource>,
    cases: Vec<BundleEntry>,
}

struct Bundle {
    sources: Vec<BundledSource>,
    cases: BTreeMap<String, Vec<Value>>,
    identity: FixtureIdentity,
}

fn load_bundle(path: &Path) -> Result<Bundle, Failure> {
    let bytes = crate::artifacts::read_bounded(path, 64 * 1024 * 1024)?;
    let input: BundleInput =
        serde_json::from_slice(&bytes).map_err(|e| invalid("fixture_invalid", &e.to_string()))?;
    if input.cases.is_empty() || input.sources.is_empty() {
        return Err(invalid(
            "fixture_invalid",
            "bundle must contain sources and cases",
        ));
    }
    if input.cases.len() > 100000 || input.sources.len() > MAX_FILES {
        return Err(invalid(
            "limit_exceeded",
            "bundle allows <= 100000 cases and <= 4096 sources",
        ));
    }
    let mut declared = BTreeSet::new();
    let mut files = BTreeSet::new();
    for source in &input.sources {
        if source.file.is_empty()
            || !files.insert(source.file.clone())
            || source.sha256.len() != 64
            || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || source.cases.is_empty()
        {
            return Err(invalid(
                "fixture_invalid",
                "invalid or duplicate bundled source",
            ));
        }
        for name in &source.cases {
            if !declared.insert((source.file.clone(), name.clone())) {
                return Err(invalid(
                    "fixture_invalid",
                    "duplicate declared bundled case",
                ));
            }
        }
    }
    let mut cases: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for entry in input.cases {
        let name = entry.case["name"]
            .as_str()
            .ok_or_else(|| invalid("fixture_invalid", "missing bundled case name"))?;
        if !declared.remove(&(entry.file.clone(), name.to_owned())) {
            return Err(invalid(
                "fixture_invalid",
                "bundled case is duplicate or missing from source provenance",
            ));
        }
        cases.entry(entry.file).or_default().push(entry.case);
    }
    if !declared.is_empty() {
        return Err(invalid(
            "fixture_invalid",
            "declared bundled cases are missing",
        ));
    }
    Ok(Bundle {
        sources: input.sources,
        cases,
        identity: FixtureIdentity {
            file: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            file_sha256: crate::artifacts::digest(&bytes),
            compressed_bytes: bytes.len(),
        },
    })
}

pub fn run(
    suite: &Path,
    filter: Option<&str>,
    max_failures: usize,
    relocate: bool,
    compare_cycles: bool,
) -> Result<Envelope<ConformanceData>, Failure> {
    run_controlled(
        suite,
        filter,
        max_failures,
        relocate,
        compare_cycles,
        &crate::context::Context::default(),
    )
}

pub fn run_controlled(
    suite: &Path,
    filter: Option<&str>,
    max_failures: usize,
    relocate: bool,
    compare_cycles: bool,
    context: &crate::context::Context,
) -> Result<Envelope<ConformanceData>, Failure> {
    context.check()?;
    if max_failures > 4096 {
        return Err(invalid(
            "limit_exceeded",
            "failure detail limit must be <= 4096",
        ));
    }
    let bundle = if suite.is_file() {
        Some(load_bundle(suite)?)
    } else {
        None
    };
    let dir = if bundle.is_some() {
        suite.to_path_buf()
    } else {
        suite.join("v1")
    };
    if bundle.is_none() && !dir.is_dir() {
        return Err(Failure {
            code: "suite_not_found".into(),
            message: format!("no v1 directory under {}", suite.display()),
            remediation: Some(
                "git clone --depth 1 https://github.com/SingleStepTests/680x0.git \
                 <path>; pass <path>/68000 (contains v1/*.json.gz), or a local bundled regression JSON file under crates/mdx/testdata"
                    .into(),
            ),
        });
    }

    let mut names: Vec<String> = if let Some(bundle) = &bundle {
        bundle.cases.keys().cloned().collect()
    } else {
        let entries: Vec<_> = fs::read_dir(&dir)
            .map_err(|e| Failure {
                code: "suite_unreadable".into(),
                message: e.to_string(),
                remediation: None,
            })?
            .take(MAX_FILES + 1)
            .collect::<Result<_, _>>()
            .map_err(|e| invalid("suite_unreadable", &e.to_string()))?;
        if entries.len() > MAX_FILES {
            return Err(invalid(
                "limit_exceeded",
                "suite directory must contain <= 4096 entries",
            ));
        }
        entries
            .into_iter()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".json.gz"))
            .collect()
    };
    names.sort();

    let selected: Vec<String> = names
        .iter()
        .filter(|n| match filter {
            Some(f) => n.contains(f),
            None => true,
        })
        .cloned()
        .collect();

    if selected.is_empty() {
        return Err(Failure {
            code: "filter_matched_nothing".into(),
            message: format!("filter {filter:?} matched none of {} files", names.len()),
            remediation: None,
        });
    }

    let mut per_file = Vec::new();
    let mut failures = Vec::new();
    let mut cases_run = 0usize;
    let mut cases_passed = 0usize;
    let mut termination = "complete";
    let mut source_files = Vec::new();

    for name in &selected {
        if cases_run >= MAX_CASES {
            termination = "case_budget";
            break;
        }
        if let Some(reason) = context.stop_reason() {
            termination = reason;
            break;
        }
        let (passed, total, available) = if let Some(bundle) = &bundle {
            run_cases(
                &bundle.cases[name],
                name,
                max_failures,
                relocate,
                compare_cycles,
                &mut failures,
                context,
                MAX_CASES - cases_run,
            )?
        } else {
            let path = dir.join(name);
            let (passed, total, available, identity) = run_file(
                &path,
                name,
                max_failures,
                relocate,
                compare_cycles,
                &mut failures,
                context,
                MAX_CASES - cases_run,
            )?;
            source_files.push(identity);
            (passed, total, available)
        };
        cases_run += total;
        cases_passed += passed;
        per_file.push(PerFile {
            file: name.trim_end_matches(".json.gz").to_string(),
            cases: total,
            passed,
            failed: total - passed,
        });
        context.report(per_file.len() as u64, selected.len() as u64);
        if total < available {
            termination = context.stop_reason().unwrap_or("case_budget");
            break;
        }
    }

    // Worst first: the response should open with what is still broken.
    per_file.sort_by(|a, b| b.failed.cmp(&a.failed).then(a.file.cmp(&b.file)));

    let bundled = bundle.is_some();
    let bundled_sources = if let Some(bundle) = bundle {
        source_files.push(bundle.identity);
        bundle.sources
    } else {
        Vec::new()
    };

    let cases_failed = cases_run - cases_passed;
    let data = ConformanceData {
        source_files,
        bundled_sources,
        termination: termination.into(),
        failure_details_omitted: cases_failed.saturating_sub(failures.len()),
        cycles_compared: compare_cycles,
        suite_path: dir.display().to_string(),
        files_run: per_file.len(),
        files_total: names.len(),
        cases_run,
        cases_passed,
        cases_failed,
        per_instruction: per_file,
        failures,
    };

    let mut warnings = vec![if compare_cycles {
        Warning::new(
            "bus_order_not_compared",
            "total instruction cycles are compared; individual bus-transaction ordering is not",
        )
    } else {
        Warning::new(
            "timing_not_compared",
            "instruction cycles and bus-transaction ordering are not compared",
        )
    }];
    if relocate {
        warnings.push(Warning::new(
            "noncanonical_fixture_relocation",
            "fixture memory was rewritten; PC and prefetch comparison are disabled",
        ));
    }

    let mut coverage = if cases_failed == 0
        && selected.len() == names.len()
        && !relocate
        && termination == "complete"
    {
        Coverage::complete(vec![format!("cases[0..{cases_run}]")])
    } else {
        Coverage::partial(
            vec![format!("cases[0..{cases_run}]")],
            if cases_failed > 0 {
                vec![format!(
                    "failure_details: retained {} of {cases_failed}",
                    data.failures.len()
                )]
            } else {
                vec![]
            },
            if selected.len() != names.len() {
                vec![format!("files[0..{}] of {}", selected.len(), names.len())]
            } else {
                vec![]
            },
        )
    };
    coverage.limits.push(format!(
        "failure details <= {max_failures}; decompressed fixture file <= 64 MiB"
    ));
    coverage.limits.push(
        "suite entries <= 4096; cases attempted <= 20000000; cases per file <= 100000".into(),
    );
    if bundled {
        coverage.conditions.push("local bundled regression subset; files_total counts bundled instruction groups, not upstream suite files".into());
        coverage.limitations.push("only bundled cases were executed; upstream suite coverage is not implied; bundled_sources hashes are declared provenance, while source_files hashes verify the actual local bundle bytes".into());
    }
    if termination != "complete" {
        coverage.limitations.push(format!(
            "conformance terminated with {termination}; {} files processed",
            data.files_run
        ));
    }

    let mut envelope = Envelope::ok(TOOL, suite_subject(&data), coverage, data);
    envelope.warnings = warnings;
    Ok(envelope)
}

fn suite_subject(data: &ConformanceData) -> Option<Subject> {
    Some(Subject {
        path: data.suite_path.clone(),
        sha256: crate::artifacts::digest(&serde_json::to_vec(&data.source_files).unwrap()),
        bytes: data
            .source_files
            .iter()
            .map(|file| file.compressed_bytes)
            .sum(),
        format: "m68000-analyzed-fixture-manifest".into(),
        architecture: Some("m68000".into()),
    })
}

fn run_file(
    path: &Path,
    name: &str,
    max_failures: usize,
    relocate: bool,
    compare_cycles: bool,
    out: &mut Vec<Failure_>,
    context: &crate::context::Context,
    remaining_cases: usize,
) -> Result<(usize, usize, usize, FixtureIdentity), Failure> {
    let mut raw = Vec::new();
    let compressed = crate::artifacts::read_bounded(path, 64 * 1024 * 1024)?;
    let identity = FixtureIdentity {
        file: name.into(),
        file_sha256: crate::artifacts::digest(&compressed),
        compressed_bytes: compressed.len(),
    };
    let mut decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(compressed));
    decoder
        .by_ref()
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| invalid("fixture_invalid", &e.to_string()))?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(invalid(
            "limit_exceeded",
            "decompressed fixture exceeds 64 MiB",
        ));
    }
    let cases: Vec<Value> =
        serde_json::from_slice(&raw).map_err(|e| invalid("fixture_invalid", &e.to_string()))?;
    let (passed, attempted, total) = run_cases(
        &cases,
        name,
        max_failures,
        relocate,
        compare_cycles,
        out,
        context,
        remaining_cases,
    )?;
    Ok((passed, attempted, total, identity))
}

fn run_cases(
    cases: &[Value],
    name: &str,
    max_failures: usize,
    relocate: bool,
    compare_cycles: bool,
    out: &mut Vec<Failure_>,
    context: &crate::context::Context,
    remaining_cases: usize,
) -> Result<(usize, usize, usize), Failure> {
    if cases.is_empty() {
        return Err(invalid("fixture_invalid", "fixture file contains no cases"));
    }
    if cases.len() > 100000 {
        return Err(invalid(
            "limit_exceeded",
            "fixture file must contain <= 100000 cases",
        ));
    }

    let mut passed = 0usize;
    let total = cases.len();
    let mut attempted = 0;
    for case in cases {
        if attempted >= remaining_cases {
            break;
        }
        if context.stop_reason().is_some() {
            break;
        }
        validate_case(case, compare_cycles)?;
        if let Some(f) = check(case, name, relocate, compare_cycles) {
            if out.len() < max_failures {
                out.push(f);
            }
        } else {
            passed += 1;
        }
        attempted += 1;
    }
    Ok((passed, attempted, total))
}

fn invalid(code: &str, message: &str) -> Failure {
    Failure {
        code: code.into(),
        message: message.into(),
        remediation: None,
    }
}

fn validate_case(case: &Value, compare_cycles: bool) -> Result<(), Failure> {
    if !case["name"].is_string() || (compare_cycles && case["length"].as_u64().is_none()) {
        return Err(invalid(
            "fixture_invalid",
            "missing fixture name or measured cycle length",
        ));
    }
    for state in ["initial", "final"] {
        for key in REGISTERS
            .into_iter()
            .map(String::from)
            .chain((0..7).map(|i| format!("a{i}")))
            .chain(["pc", "sr", "usp", "ssp"].into_iter().map(String::from))
        {
            if !case[state][&key]
                .as_u64()
                .is_some_and(|n| n <= u32::MAX as u64)
            {
                return Err(invalid(
                    "fixture_invalid",
                    &format!("missing or invalid {state}.{key}"),
                ));
            }
        }
        if !case[state]["prefetch"].as_array().is_some_and(|a| {
            a.len() == 2 && a.iter().all(|v| v.as_u64().is_some_and(|n| n <= 65535))
        }) {
            return Err(invalid(
                "fixture_invalid",
                "prefetch must contain two words",
            ));
        }
        if !case[state]["ram"].as_array().is_some_and(|a| {
            a.iter().all(|v| {
                v.as_array().is_some_and(|p| {
                    p.len() == 2
                        && p[0].as_u64().is_some_and(|n| n <= u32::MAX as u64)
                        && p[1].as_u64().is_some_and(|n| n <= 255)
                })
            })
        }) {
            return Err(invalid(
                "fixture_invalid",
                "RAM entries must be [address, byte] pairs",
            ));
        }
    }
    Ok(())
}

/// Run one case and return the first mismatch, if any.
fn check(case: &Value, file: &str, relocate: bool, compare_cycles: bool) -> Option<Failure_> {
    let name = case.get("name").and_then(Value::as_str).unwrap_or("?");
    let initial = case.get("initial")?;
    let expected = case.get("final")?;
    let opcode = initial
        .get("prefetch")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_u64)
        .map(|w| format!("0x{w:04X}"))
        .unwrap_or_else(|| "????".into());

    let mut cpu = M68000::new(initial.get("pc").and_then(Value::as_u64).unwrap_or(0) as u32);
    cpu.hardware_prefetch = true;
    for (i, r) in REGISTERS.iter().enumerate() {
        cpu.d[i] = num(initial, r);
        let an = format!("a{i}");
        cpu.a[i] = num(initial, &an);
    }
    cpu.usp = num(initial, "usp");
    let ssp = num(initial, "ssp");
    cpu.ssp = ssp;
    cpu.sr = StatusRegister(num(initial, "sr") as u16);
    cpu.a[7] = if cpu.sr.is_supervisor() { ssp } else { cpu.usp };
    let pf = initial.get("prefetch").and_then(Value::as_array);
    if let Some(pf) = pf {
        cpu.prefetch[0] = pf.first().and_then(Value::as_u64).unwrap_or(0) as u16;
        cpu.prefetch[1] = pf.get(1).and_then(Value::as_u64).unwrap_or(0) as u16;
    }

    let mut bus = SparseBus::new();
    if let Some(ram) = initial.get("ram").and_then(Value::as_array) {
        for entry in ram {
            if let Some(pair) = entry.as_array() {
                let addr = pair.first().and_then(Value::as_u64).unwrap_or(0) as u32;
                let value = pair.get(1).and_then(Value::as_u64).unwrap_or(0);
                bus.set_byte(addr, value as u8);
            }
        }
    }
    // Retained only for reproducing historical diagnostic runs. The fixture's
    // second prefetch word is already at PC+2; shifting RAM down from PC+4
    // overwrites that word and corrupts the supplied instruction.
    if relocate {
        let pc = initial.get("pc").and_then(Value::as_u64).unwrap_or(0) as u32;
        // Read the whole instruction area first: relocating downwards would
        // otherwise overwrite a word still to be copied.
        let mut words = [(0u8, 0u8); 5];
        for (k, slot) in words.iter_mut().enumerate() {
            let src = pc + 4 + k as u32 * 2;
            *slot = (bus.get(src).unwrap_or(0), bus.get(src + 1).unwrap_or(0));
        }
        bus.set_byte(pc, (cpu.prefetch[0] >> 8) as u8);
        bus.set_byte(pc + 1, (cpu.prefetch[0] & 0xFF) as u8);
        for (k, (hi, lo)) in words.iter().enumerate() {
            if *hi == 0 && *lo == 0 {
                continue;
            }
            bus.set_byte(pc + 2 + k as u32 * 2, *hi);
            bus.set_byte(pc + 3 + k as u32 * 2, *lo);
        }
        cpu.prefetch[0] = bus.read16(pc);
        cpu.prefetch[1] = bus.read16(pc + 2);
    }

    let outcome = cpu.step(&mut bus);

    let mismatch = |field: &str, want: u32, got: u32| {
        Some(Failure_ {
            file: file.to_string(),
            case: name.to_string(),
            opcode: opcode.clone(),
            field: field.to_string(),
            expected: format!("0x{want:X}"),
            actual: format!("0x{got:X}"),
        })
    };

    for r in REGISTERS.iter() {
        let want = num(expected, r);
        if cpu.d[reg_index(r)] != want {
            return mismatch(r, want, cpu.d[reg_index(r)]);
        }
    }
    for i in 0..6 {
        let key = format!("a{i}");
        let want = num(expected, &key);
        if cpu.a[i] != want {
            return mismatch(&key, want, cpu.a[i]);
        }
    }

    let want_sr = num(expected, "sr") as u16;
    if cpu.sr.0 != want_sr {
        return mismatch("sr", want_sr as u32, cpu.sr.0 as u32);
    }
    let want_a7 = if cpu.sr.is_supervisor() {
        num(expected, "ssp")
    } else {
        num(expected, "usp")
    };
    if cpu.a[7] != want_a7 {
        return mismatch("a7", want_a7, cpu.a[7]);
    }
    if cpu.sr.is_supervisor() && cpu.usp != num(expected, "usp") {
        return mismatch("usp", num(expected, "usp"), cpu.usp);
    }

    if !relocate {
        let want_pc = num(expected, "pc");
        if cpu.pc != want_pc {
            return mismatch("pc", want_pc, cpu.pc);
        }
        if let Some(words) = expected.get("prefetch").and_then(Value::as_array) {
            for (i, word) in words.iter().take(2).enumerate() {
                let want = word.as_u64().unwrap_or(0) as u32;
                if cpu.prefetch[i] as u32 != want {
                    return mismatch(&format!("prefetch[{i}]"), want, cpu.prefetch[i] as u32);
                }
            }
        }
    }

    if let Some(ram) = expected.get("ram").and_then(Value::as_array) {
        for entry in ram {
            if let Some(pair) = entry.as_array() {
                let addr = pair.first().and_then(Value::as_u64).unwrap_or(0) as u32;
                let want = pair.get(1).and_then(Value::as_u64).unwrap_or(0) as u8;
                let got = bus.get(addr).unwrap_or(0);
                if got != want {
                    return mismatch(&format!("ram[0x{addr:X}]"), want as u32, got as u32);
                }
            }
        }
    }
    if compare_cycles && outcome.cycles != num(case, "length") {
        return mismatch("cycles", num(case, "length"), outcome.cycles);
    }
    None
}

fn reg_index(name: &str) -> usize {
    name[1..].parse().unwrap_or(0)
}

fn num(v: &Value, key: &str) -> u32 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0) as u32
}

pub fn suite_path(arg: &Path) -> PathBuf {
    arg.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_boot_regressions_preserve_fixture_inputs() {
        for source in [
            include_str!("../../testdata/cpu-regressions.json"),
            include_str!("../../testdata/pea-regressions.json"),
            include_str!("../../testdata/alu-regressions.json"),
            include_str!("../../testdata/exg-regressions.json"),
        ] {
            let fixtures: Value =
                serde_json::from_str(source).expect("unaltered SingleStepTests fixtures");
            for entry in fixtures["cases"].as_array().unwrap() {
                let failure = check(&entry["case"], entry["file"].as_str().unwrap(), false, true);
                assert!(failure.is_none(), "reference mismatch: {failure:?}");
            }
        }
    }
}
