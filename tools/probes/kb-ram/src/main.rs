use mdcore::{Cartridge, System};
use std::{fs, io::{BufWriter, Write}};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut s = System::new(Cartridge::load(&args[1])?);
    s.observation_limit = 0;
    let frames: u64 = args[2].parse()?;
    let prefix = &args[3];
    let replay: serde_json::Value = serde_json::from_slice(&fs::read(&args[4])?)?;
    let events = replay["events"].as_array().unwrap();
    let mut next_input = 0;
    let mut log = BufWriter::new(fs::File::create(format!("{prefix}.jsonl"))?);
    let mut steps = 0;
    let mut rng_calls = 0;
    let mut irq_calls = 0;
    let mut samples = Vec::new();
    let mut saved = s.wram[0x8ac8..0x8acc].to_vec();
    // Diagnostic data-bus counterfactual, not an emulator fix: substitute
    // independently observed HV/tick responses, asserting the exact read order.
    let hv: Vec<(u32,u32)> = if let Some(path) = args.get(5).filter(|p| *p!="-") {
        fs::read_to_string(path)?.lines().map(serde_json::from_str::<serde_json::Value>).collect::<Result<Vec<_>,_>>()?.into_iter()
            .filter(|r| (r["kind"] == "hv" && matches!(r["pc"].as_u64(),Some(0x1122|0x126a|0x13b24|0x13b5e))) || (args.get(6).is_some_and(|x| x=="ticks") && r["kind"]=="tick"))
            .map(|r| (r["pc"].as_u64().unwrap() as u32,r["value"].as_u64().unwrap() as u32)).collect()
    } else {Vec::new()};
    let mut next_hv = 0;
    let timing_start = args.get(7).map(|x| x.parse::<u64>()).transpose()?.unwrap_or(912);
    let timing_limit = args.get(8).map(|x| x.parse::<u64>()).transpose()?.unwrap_or(u64::MAX);
    let mut timing_records = 0;
    while s.vdp.frame < frames {
        // Separate diagnostic of the missing VDP -> Z80 interrupt wire. Model
        // the oracle's asserted portion of line 224 as a level, not a RAM edit.
        if args.get(6).is_some_and(|x|x=="z80-vint") {
            let edge=if s.vdp.registers[12]&1!=0 {788} else {770};
            s.z80.irq_pending=s.vdp.line==224 && s.vdp.dot*3420/896>=edge;
        }
        while next_input < events.len() && events[next_input]["frame"].as_u64().unwrap() <= s.vdp.frame {
            let e = &events[next_input];
            let mut mask = 0;
            for b in e["buttons"].as_array().unwrap() {
                mask |= 1 << ["up", "down", "left", "right", "a", "b", "c", "start"].iter().position(|x| Some(*x) == b.as_str()).unwrap();
            }
            s.io.pad_buttons[e["port"].as_u64().unwrap_or(0) as usize] = mask;
            next_input += 1;
        }
        let pc = s.cpu.pc;
        let irq = s.irq_level().is_some_and(|l| s.cpu.sr.accepts(l));
        if !irq && pc == 0x13b0c {rng_calls += 1;}
        if !irq && pc == 0x2a060 {irq_calls += 1;}
        let timing_trace = args.get(6).is_some_and(|x| x == "timing") && s.vdp.frame >= timing_start && timing_records < timing_limit;
        if timing_trace { timing_records += 1; }
        let selected = timing_trace || (!irq && (matches!(pc, 0x111e|0x1122|0x1124|0x1128|0x112c|0x126a|0x1272|0x13b0c|0x13b44|0x13b78|0x13bb4|0x29b86|0x29de0|0x29de4|0x29de8|0x29dec|0x29df2|0x29e02)
            || (pc == 0x2a060 && irq_calls <= 12)
            || ((0x13b0c..0x13bba).contains(&pc) && rng_calls <= 6)));
        let pre = if selected {Some(serde_json::json!({"step":steps,"pc":pc,"master":s.master_cycles(),"irq":irq,"frame":s.vdp.frame,"line":s.vdp.line,"dot":s.vdp.dot,"d":s.cpu.d,"a":s.cpu.a,"sr":s.cpu.sr.0,"rng_call":rng_calls,"seed":s.wram[0xff46..0xff4c],"timer":s.wram[0xe4ea..0xe4ee],"sound_flags":[s.z80_ram[0xb],s.z80_ram[0xda]],"z80_pc":s.z80.pc,"z80_iff1":s.z80.iff1}))} else {None};
        let (cycles, ex) = s.step_instruction();
        if !irq && !hv.is_empty() && (matches!(pc,0x1122|0x126a|0x13b24|0x13b5e) || (args.get(6).is_some_and(|x|x=="ticks") && matches!(pc,0x13b44|0x13b78))) {
            let (expected,value) = *hv.get(next_hv).ok_or("HV stream exhausted")?;
            if expected != pc {return Err(format!("HV order differs at {next_hv}: {pc:x} != {expected:x}").into());}
            let reg=if pc==0x126a {0} else if pc==0x13b78 {2} else {1};
            let long=matches!(pc,0x13b44|0x13b78);
            let result=if pc==0x13b44 {value&0xffff} else {value};
            s.cpu.d[reg] = if long {result} else {(s.cpu.d[reg] & 0xffff0000) | result};
            s.cpu.sr.0 = (s.cpu.sr.0 & !15) | if result==0 {4} else if result & if long {0x80000000} else {0x8000} !=0 {8} else {0};
            next_hv += 1;
        }
        if let Some(mut row) = pre {
            row["master_after"] = serde_json::json!(s.master_cycles());
            row["cycles"] = serde_json::json!(cycles);
            row["post_d"] = serde_json::json!(s.cpu.d);
            row["post_a"] = serde_json::json!(s.cpu.a);
            row["post_seed"] = serde_json::json!(s.wram[0xff46..0xff4c]);
            writeln!(log, "{row}")?;
        }
        if saved != s.wram[0x8ac8..0x8acc] {
            writeln!(log,"{}",serde_json::json!({"kind":"saved_change","pc":pc,"step":steps,"frame":s.vdp.frame,"old":saved,"new":s.wram[0x8ac8..0x8acc]}))?;
            saved = s.wram[0x8ac8..0x8acc].to_vec();
        }
        if let Some(ex) = ex {return Err(format!("exception {ex:?} at {pc:x}").into());}
        steps += 1;
        if [650,915,1800,4000,4100,4500,6000].contains(&s.vdp.frame) && samples.last() != Some(&s.vdp.frame) {
            samples.push(s.vdp.frame);
            fs::write(format!("{prefix}-{}.wram",s.vdp.frame), &s.wram)?;
        }
    }
    fs::write(format!("{prefix}.wram"), &s.wram)?;
    fs::write(format!("{prefix}.vram"), &s.vdp.vram)?;
    fs::write(format!("{prefix}.cram"), &s.vdp.cram)?;
    fs::write(format!("{prefix}.vsram"), &s.vdp.vsram)?;
    fs::write(format!("{prefix}.registers.json"),serde_json::to_vec(&s.vdp.registers.to_vec())?)?;
    let (fb,_) = s.vdp.render();
    fs::write(format!("{prefix}.rgb"),fb.data)?;
    println!("{}",serde_json::json!({"frames":s.vdp.frame,"steps":steps,"pc":s.cpu.pc,"rng_calls":rng_calls,"hv_substitutions":next_hv,"hv_available":hv.len(),"seed":s.wram[0xff46..0xff4c],"rom_sha256":s.cart.sha256}));
    Ok(())
}
