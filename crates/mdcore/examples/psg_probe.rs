//! Authored pulse/phase observer paired with the unchanged external PSG core.
use mdcore::psg::Psg;
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut psg = Psg::new();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let v: Vec<i64> = line?
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        let edges = if v[1] == -2 {
            psg = Psg::new();
            Vec::new()
        } else if v[1] == -1 {
            psg.advance_to(v[0] as u64)
        } else {
            psg.write(v[0] as u64, v[1] as u8)
        };
        let mut combined = BTreeMap::<u64, i32>::new();
        for edge in edges {
            *combined.entry(edge.master).or_default() += edge.delta;
        }
        let edges: Vec<_> = combined
            .into_iter()
            .filter(|(_, delta)| *delta != 0)
            .map(|(at, delta)| serde_json::json!([at, delta, delta]))
            .collect();
        writeln!(
            out,
            "{}",
            serde_json::json!({"edges":edges,"phase":psg.phase()})
        )?;
    }
    Ok(())
}
