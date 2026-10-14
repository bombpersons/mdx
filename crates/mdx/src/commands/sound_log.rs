//! Shared opt-in log of 68000 writes into the sound side (Z80 RAM, bank
//! register, BUSREQ, RESET) for `boot` and `audio`. Records go to a JSONL
//! artifact; the response carries counts and a per-address summary.
use crate::artifacts::{self, Artifact};
use crate::envelope::Failure;
use mdcore::observe::{SoundCommandLog, SoundCommandTarget};
use mdcore::System;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

fn max_records() -> usize {
    100_000
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundCommandOptions {
    /// Records retained in the artifact; counts continue past the cap.
    #[serde(default = "max_records")]
    #[schemars(range(min = 1, max = 1000000))]
    pub max_records: usize,
    /// Only log Z80 RAM writes at offsets z80_start..z80_start+z80_length
    /// (e.g. a driver's command slots). Control writes are always logged.
    pub z80_start: Option<u16>,
    pub z80_length: Option<u16>,
    /// Also record each BUSREQ/RESET write (games toggle BUSREQ many times
    /// per frame); otherwise they are only counted.
    #[serde(default)]
    pub bus_control: bool,
}

impl Default for SoundCommandOptions {
    fn default() -> Self {
        Self { max_records: max_records(), z80_start: None, z80_length: None, bus_control: false }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Z80AddressSummary {
    pub z80_address: String,
    pub writes: u64,
    /// Distinct frames with a write, and the last of them.
    pub frames: u64,
    pub last_frame: u64,
    /// Up to 32 distinct values in first-seen order.
    pub values: Vec<String>,
    /// Up to 8 distinct writing PCs in first-seen order.
    pub writer_pcs: Vec<String>,
}

/// A frame with at least 32 Z80 RAM writes: a driver or data
/// upload rather than commands. Its writes are left out of the busiest list.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Upload {
    pub frame: u64,
    pub writes: u64,
    pub first_z80_address: String,
    pub last_z80_address: String,
}

const UPLOAD_WRITES: u64 = 32;

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SoundCommandData {
    /// JSONL: frame, master_cycle, pc, target, address, z80_address, value,
    /// z80_bus_granted. Frames are absolute.
    pub records_artifact: Artifact,
    pub observed: u64,
    pub retained: usize,
    pub omitted: u64,
    pub by_target: BTreeMap<String, u64>,
    /// BUSREQ/RESET writes counted but not recorded (see `bus_control`).
    pub bus_control_counted: u64,
    pub uploads: Vec<Upload>,
    /// Up to 32 Z80 RAM addresses outside uploads, ranked by distinct frames
    /// written, then writes, then recency. Driver command slots usually lead.
    pub busiest_z80_addresses: Vec<Z80AddressSummary>,
}

pub fn validate(options: &SoundCommandOptions) -> Result<(), Failure> {
    let range_ok = match (options.z80_start, options.z80_length) {
        (None, None) => true,
        (Some(start), Some(length)) => length > 0 && u32::from(start) + u32::from(length) <= 0x2000,
        _ => false,
    };
    if options.max_records == 0 || options.max_records > 1_000_000 || !range_ok {
        return Err(artifacts::failure(
            "invalid_arguments",
            "sound_commands needs max_records 1..1000000 and, if given, both z80_start and z80_length inside 0x0000..0x2000",
        ));
    }
    Ok(())
}

/// Start logging on `system`.
pub fn install(system: &mut System, options: &SoundCommandOptions) {
    let range = options.z80_start.zip(options.z80_length);
    system.sound_commands = Some(SoundCommandLog::new(options.max_records, range, options.bus_control));
}

/// Stop logging and publish the records.
pub fn finish(system: &mut System, store: &Path) -> Result<Option<SoundCommandData>, Failure> {
    let Some(log) = system.sound_commands.take() else {
        return Ok(None);
    };
    let mut encoded = Vec::new();
    let mut by_target = BTreeMap::new();
    // writes, values, writer PCs, distinct frames, last frame
    let mut addresses: BTreeMap<u16, (u64, Vec<u8>, Vec<u32>, u64, u64)> = BTreeMap::new();
    let mut per_frame: BTreeMap<u64, (u64, u16, u16)> = BTreeMap::new();
    for record in &log.records {
        if let Some(at) = record.z80_address {
            let entry = per_frame.entry(record.frame).or_insert((0, at, at));
            entry.0 += 1;
            entry.1 = entry.1.min(at);
            entry.2 = entry.2.max(at);
        }
    }
    let upload_frames: std::collections::BTreeSet<u64> = per_frame
        .iter()
        .filter(|(_, (writes, _, _))| *writes >= UPLOAD_WRITES)
        .map(|(frame, _)| *frame)
        .collect();
    for record in &log.records {
        serde_json::to_writer(&mut encoded, record).unwrap();
        encoded.push(b'\n');
        let name = serde_json::to_value(record.target).unwrap();
        *by_target.entry(name.as_str().unwrap().to_string()).or_insert(0) += 1;
        if upload_frames.contains(&record.frame) {
            continue;
        }
        if let (SoundCommandTarget::Z80Ram, Some(at)) = (record.target, record.z80_address) {
            let entry = addresses.entry(at).or_default();
            entry.0 += 1;
            if entry.3 == 0 || entry.4 != record.frame {
                entry.3 += 1;
                entry.4 = record.frame;
            }
            if entry.1.len() < 32 && !entry.1.contains(&record.value) {
                entry.1.push(record.value);
            }
            if entry.2.len() < 8 && !entry.2.contains(&record.pc) {
                entry.2.push(record.pc);
            }
        }
    }
    let mut busiest: Vec<_> = addresses.into_iter().collect();
    // Command slots are rewritten across many frames; ties favour recent writes.
    busiest.sort_by(|a, b| (b.1.3, b.1.0, b.1.4).cmp(&(a.1.3, a.1.0, a.1.4)).then(a.0.cmp(&b.0)));
    let busiest_z80_addresses = busiest
        .into_iter()
        .take(32)
        .map(|(at, (writes, values, pcs, frames, last_frame))| Z80AddressSummary {
            z80_address: format!("0x{at:04X}"),
            writes,
            frames,
            last_frame,
            values: values.iter().map(|v| format!("0x{v:02X}")).collect(),
            writer_pcs: pcs.iter().map(|pc| format!("0x{pc:06X}")).collect(),
        })
        .collect();
    Ok(Some(SoundCommandData {
        records_artifact: artifacts::put(store, &encoded)?,
        observed: log.observed,
        retained: log.records.len(),
        omitted: log.observed - log.records.len() as u64,
        by_target,
        bus_control_counted: log.bus_control_counted,
        uploads: upload_frames
            .iter()
            .map(|frame| {
                let (writes, first, last) = per_frame[frame];
                Upload {
                    frame: *frame,
                    writes,
                    first_z80_address: format!("0x{first:04X}"),
                    last_z80_address: format!("0x{last:04X}"),
                }
            })
            .collect(),
        busiest_z80_addresses,
    }))
}
