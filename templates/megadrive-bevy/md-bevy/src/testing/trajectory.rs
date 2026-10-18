//! Trajectory tests: replay the original's input in the rebuild and compare
//! recorded values tick by tick.
//!
//! Record the original once with mdx, from a saved state, sampling the RAM
//! the feature's state lives in (mdx `boot` with `input` and `samples`):
//!
//! ```text
//! mdx boot --rom game.md --from-state walk.state.json --frames 60 \
//!     --input tests/data/walk.input.json \
//!     --sample 0xFFD008:s16:x --sample 0xFFD00C:s16:y
//! cp <data.samples.artifact.path> tests/data/walk.samples.csv
//! ```
//!
//! Then a test spawns the recorded starting state, replays the same input,
//! records the same values in the original's units and compares:
//!
//! ```
//! use bevy::prelude::*;
//! use md_bevy::prelude::*;
//! use md_bevy::testing::{Alignment, Replay, Trajectory, compare, headless_app};
//!
//! # #[derive(Component)] struct X(i32);
//! # fn walk(pad: Res<ControllerState>, mut q: Query<&mut X>) {
//! #     for mut x in &mut q { x.0 += 2 * i32::from(pad.held(Button::Right)); }
//! # }
//! // In a test: Replay::load("tests/data/walk.input.json") and
//! // Trajectory::load_mdx_samples("tests/data/walk.samples.csv", align).
//! let align = Alignment::new(1, 0); // in a game: Alignment::new(game::FRAMES_PER_TICK, 0)
//! let replay = Replay::parse(r#"{"events":[{"frame":0,"buttons":["right"]},{"frame":3,"buttons":[]}]}"#).unwrap();
//! let original = Trajectory::parse_mdx_samples(
//!     "run_frame,frame,x\n0,900,10\n1,901,12\n2,902,14\n3,903,16\n4,904,16\n",
//!     align,
//! )
//! .unwrap();
//!
//! let mut app = headless_app(TickRate::every_frame(Region::Ntsc));
//! app.add_systems(FixedUpdate, walk.in_set(GameSystems::Game));
//! let player = app.world_mut().spawn(X(original.get(0, "x").unwrap() as i32)).id();
//! let inputs = replay.tick_inputs(align, original.ticks() - 1).unwrap();
//! let rebuild = Trajectory::record(&mut app, &inputs, &["x"], |world| {
//!     vec![world.get::<X>(player).unwrap().0 as f64]
//! });
//! compare(&original, &rebuild, &[("x", 0.0)]).unwrap().assert_passed();
//! ```
//!
//! Timing: sample row N is the original's memory when video frame N of the
//! run begins (the start of vertical blank), before frame N's input takes
//! effect; row 0 is the starting state. Tick k of the rebuild therefore uses
//! the input held at frame `start + k * frames_per_tick`, and the state after
//! k ticks is compared with that row ([`Alignment`]). Record the rebuild in
//! the original's units (fixed point decoded, screen axes), so each value
//! compares directly.
use crate::controller::Button;
use crate::testing::TickApp;
use bevy::prelude::*;
use std::fmt;
use std::path::Path;

/// How the rebuild's ticks line up with the original's video frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alignment {
    /// Frames per logic tick: the game's `FRAMES_PER_TICK`.
    pub frames_per_tick: u32,
    /// The sample row (run frame) that is the state before the first tick.
    pub start: u32,
}

impl Alignment {
    pub const fn new(frames_per_tick: u32, start: u32) -> Self {
        assert!(frames_per_tick > 0, "frames_per_tick must be at least 1");
        Self { frames_per_tick, start }
    }

    /// The run frame whose sample row is the state after `ticks` ticks, and
    /// whose held input drives tick `ticks`.
    pub fn frame(self, ticks: usize) -> u64 {
        self.start as u64 + ticks as u64 * self.frames_per_tick as u64
    }
}

/// An mdx controller replay (the `input` file of `boot`): each event is the
/// complete held state of its port from its `frame`, until the port's next
/// event. Frames count video frames from the start of the run. Only port 0
/// (the template's one controller) is replayed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Replay {
    events: Vec<(u64, Vec<Button>)>,
}

impl Replay {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(json: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let object = value.as_object().ok_or("a replay is a JSON object with \"events\"")?;
        if object.get("pokes").and_then(|p| p.as_array()).is_some_and(|p| !p.is_empty()) {
            return Err("the replay pokes memory, which the rebuild cannot repeat; record from a saved state instead".into());
        }
        let mut events: Vec<(u64, Vec<Button>)> = Vec::new();
        for (i, event) in object.get("events").and_then(|e| e.as_array()).into_iter().flatten().enumerate() {
            let frame = event["frame"].as_u64().ok_or(format!("event {i} needs a frame"))?;
            let buttons: Vec<Button> = serde_json::from_value(event["buttons"].clone())
                .map_err(|e| format!("event {i}: {e}"))?;
            if event.get("port").and_then(|p| p.as_u64()).unwrap_or(0) != 0 {
                if buttons.is_empty() {
                    continue;
                }
                return Err(format!("event {i} presses buttons on another port; the rebuild replays port 0 only"));
            }
            if events.last().is_some_and(|(previous, _)| frame <= *previous) {
                return Err(format!("event {i}: frames must increase"));
            }
            events.push((frame, buttons));
        }
        Ok(Self { events })
    }

    /// The buttons held during `frame`. Before the first event nothing is held.
    pub fn held(&self, frame: u64) -> &[Button] {
        self.events.iter().rev().find(|(f, _)| *f <= frame).map_or(&[], |(_, b)| b)
    }

    /// The buttons for each of `ticks` ticks. Input may only change on a tick
    /// boundary; an edge inside a tick is an error rather than a guess about
    /// which frame the original read the pad on.
    pub fn tick_inputs(&self, align: Alignment, ticks: usize) -> Result<Vec<Vec<Button>>, String> {
        let (first, last) = (align.frame(0), align.frame(ticks));
        for &(edge, _) in self.events.iter().filter(|(f, _)| *f > first && *f < last) {
            if (edge - first) % align.frames_per_tick as u64 != 0 {
                return Err(format!(
                    "input changes at frame {edge}, inside a tick; with {} frames per tick, change input only at frames {first} + n*{}",
                    align.frames_per_tick, align.frames_per_tick
                ));
            }
        }
        Ok((0..ticks).map(|k| self.held(align.frame(k)).to_vec()).collect())
    }
}

/// Named columns of values, one row per tick (row 0 is the starting state).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Trajectory {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<f64>>,
}

impl Trajectory {
    pub fn new(columns: &[&str]) -> Self {
        Self { columns: columns.iter().map(|c| c.to_string()).collect(), rows: Vec::new() }
    }

    pub fn push(&mut self, row: Vec<f64>) {
        assert_eq!(row.len(), self.columns.len(), "row width must match the columns");
        self.rows.push(row);
    }

    /// Rows recorded: the starting state plus one per tick.
    pub fn ticks(&self) -> usize {
        self.rows.len()
    }

    pub fn get(&self, row: usize, column: &str) -> Option<f64> {
        let at = self.columns.iter().position(|c| c == column)?;
        self.rows.get(row).map(|r| r[at])
    }

    /// Records `read` before the first tick and after each tick of `inputs`.
    pub fn record(
        app: &mut App,
        inputs: &[Vec<Button>],
        columns: &[&str],
        mut read: impl FnMut(&World) -> Vec<f64>,
    ) -> Self {
        let mut out = Self::new(columns);
        out.push(read(app.world()));
        for buttons in inputs {
            app.run_tick(buttons);
            out.push(read(app.world()));
        }
        out
    }

    pub fn load_mdx_samples(path: impl AsRef<Path>, align: Alignment) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse_mdx_samples(&text, align).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// An mdx samples CSV (`run_frame,frame,<label>...`), keeping the rows
    /// at tick boundaries: run frames `start`, `start + frames_per_tick`, ...
    /// until the first missing one.
    pub fn parse_mdx_samples(text: &str, align: Alignment) -> Result<Self, String> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header: Vec<&str> = lines.next().ok_or("empty samples file")?.split(',').collect();
        if header.len() < 3 || header[0] != "run_frame" || header[1] != "frame" {
            return Err("expected a header run_frame,frame,<labels>".into());
        }
        let mut out = Self::new(&header[2..]);
        let mut by_frame = std::collections::BTreeMap::new();
        for (n, line) in lines.enumerate() {
            let cells: Vec<&str> = line.split(',').collect();
            if cells.len() != header.len() {
                return Err(format!("row {} has {} cells, expected {}", n + 1, cells.len(), header.len()));
            }
            let run_frame: u64 = cells[0].parse().map_err(|_| format!("row {}: bad run_frame", n + 1))?;
            let values = cells[2..]
                .iter()
                .map(|c| c.trim().parse::<f64>().map_err(|_| format!("row {}: bad value {c:?}", n + 1)))
                .collect::<Result<Vec<_>, _>>()?;
            by_frame.insert(run_frame, values);
        }
        let mut tick = 0;
        while let Some(values) = by_frame.remove(&align.frame(tick)) {
            out.push(values);
            tick += 1;
        }
        if out.rows.is_empty() {
            return Err(format!("no sample row at run frame {}", align.start));
        }
        Ok(out)
    }

    pub fn to_csv(&self) -> String {
        let mut out = format!("tick,{}\n", self.columns.join(","));
        for (tick, row) in self.rows.iter().enumerate() {
            let cells: Vec<String> = row.iter().map(|v| v.to_string()).collect();
            out += &format!("{tick},{}\n", cells.join(","));
        }
        out
    }
}

/// The first tick at which one column differs by more than its tolerance.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    pub column: String,
    pub tick: usize,
    pub original: f64,
    pub rebuild: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub ticks: usize,
    pub divergences: Vec<Divergence>,
}

/// Compares the listed columns over the ticks both trajectories have.
pub fn compare(original: &Trajectory, rebuild: &Trajectory, tolerances: &[(&str, f64)]) -> Result<Comparison, String> {
    let ticks = original.ticks().min(rebuild.ticks());
    let mut divergences = Vec::new();
    for &(column, tolerance) in tolerances {
        let missing = |side: &str| format!("the {side} trajectory has no column {column:?}");
        original.get(0, column).ok_or_else(|| missing("original"))?;
        rebuild.get(0, column).ok_or_else(|| missing("rebuild"))?;
        for tick in 0..ticks {
            let (a, b) = (original.get(tick, column).unwrap(), rebuild.get(tick, column).unwrap());
            if (a - b).abs() > tolerance {
                divergences.push(Divergence { column: column.into(), tick, original: a, rebuild: b });
                break;
            }
        }
    }
    Ok(Comparison { ticks, divergences })
}

impl Comparison {
    pub fn passed(&self) -> bool {
        self.divergences.is_empty()
    }

    pub fn assert_passed(&self) {
        assert!(self.passed(), "{self}");
    }

    pub fn first_divergence(&self) -> Option<usize> {
        self.divergences.iter().map(|d| d.tick).min()
    }
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.passed() {
            return write!(f, "{} ticks match", self.ticks);
        }
        writeln!(f, "trajectories diverge (compared {} ticks):", self.ticks)?;
        for d in &self.divergences {
            writeln!(f, "  {} at tick {}: original {}, rebuild {}", d.column, d.tick, d.original, d.rebuild)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_follow_events_and_refuse_mid_tick_edges() {
        let replay =
            Replay::parse(r#"{"events":[{"frame":2,"port":0,"buttons":["a"]},{"frame":4,"buttons":[]}]}"#).unwrap();
        let inputs = replay.tick_inputs(Alignment::new(2, 0), 3).unwrap();
        assert_eq!(inputs, vec![vec![], vec![Button::A], vec![]]);
        assert!(replay.tick_inputs(Alignment::new(3, 0), 2).unwrap_err().contains("inside a tick"));
        assert!(Replay::parse(r#"{"events":[],"pokes":[{"frame":0}]}"#).is_err());
    }

    #[test]
    fn samples_keep_tick_rows_and_comparisons_report_the_first_divergence() {
        let text = "run_frame,frame,x\n0,10,1\n1,11,2\n2,12,3\n3,13,4\n4,14,5\n";
        let original = Trajectory::parse_mdx_samples(text, Alignment::new(2, 0)).unwrap();
        assert_eq!(original.rows, vec![vec![1.0], vec![3.0], vec![5.0]]);
        let mut rebuild = Trajectory::new(&["x"]);
        for x in [1.0, 3.0, 6.0] {
            rebuild.push(vec![x]);
        }
        let result = compare(&original, &rebuild, &[("x", 0.0)]).unwrap();
        assert_eq!(result.first_divergence(), Some(2));
        assert!(compare(&original, &rebuild, &[("y", 0.0)]).is_err());
    }
}
