use mdcore::{Cartridge, System};
use std::{fs::File, io::{BufWriter, Write}};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let count: u64 = args[2].parse()?;
    let mut s = System::new(Cartridge::load(&args[1])?);
    let refresh = args.get(4).is_some_and(|v| v.contains("refresh"));
    let aligned = args.get(4).is_some_and(|v| v.contains("aligned"));
    let fifo = args.get(4).is_some_and(|v| v.contains("fifo"));
    let vint_delay = args.get(4).is_some_and(|v| v.contains("vint"));
    let mut deferred_vint=false;
    let trace_limit=if args.get(4).is_some_and(|v|v.contains("quiet")){0}else{2_000_000};
    let mut fifo_cycles=[0u64;4]; let mut fifo_idx=0usize;
    let mut fifo_waits=0u64;
    let target_frame: u64 = args.get(5).map(|v|v.parse()).transpose()?.unwrap_or(u64::MAX);
    let replay: serde_json::Value = args.get(6).map(|p|std::fs::read(p).map(|b|serde_json::from_slice(&b).unwrap())).transpose()?.unwrap_or(serde_json::json!({"events":[]}));
    let inputs=replay["events"].as_array().unwrap(); let mut next_input=0;
    let mut master = if aligned {784462u64} else {0};
    if aligned {s.vdp.advance_master_cycles(master as u32);}
    let mut refresh_at = (master / 896 + 1) * 896;
    let mut refresh_delays = 0u64;
    s.observation_limit = 0;
    let mut out = BufWriter::new(File::create(format!("{}.cpu", args[3]))?);
    let mut timing = BufWriter::new(File::create(format!("{}.timing", args[3]))?);
    let mut events = BufWriter::new(File::create(format!("{}.events.jsonl", args[3]))?);
    let mut ordinal = 0u64;
    let mut steps = 0u64;
    while ordinal < count && s.vdp.frame < target_frame {
        while next_input<inputs.len() && inputs[next_input]["frame"].as_u64().unwrap()<=s.vdp.frame {
            let event=&inputs[next_input];let port=event["port"].as_u64().unwrap_or(0) as usize;
            let mut mask=0u8;
            for b in event["buttons"].as_array().unwrap() {mask|=1<<["up","down","left","right","a","b","c","start"].iter().position(|x|Some(*x)==b.as_str()).unwrap();}
            s.io.pad_buttons[port]=mask; next_input+=1;
        }
        if vint_delay && deferred_vint {
            let delay=if s.vdp.registers[12]&1!=0 {788}else{770};
            if s.vdp.line==224 && s.vdp.dot*3420/896<delay {s.vdp.vint_pending=false;}
            else {s.vdp.vint_pending=true;deferred_vint=false;}
        }
        if s.irq_level().is_some_and(|l| s.cpu.sr.accepts(l)) {
            writeln!(events,"{}",serde_json::json!({"kind":"irq","ordinal":ordinal,"frame":s.vdp.frame,"line":s.vdp.line,"dot":s.vdp.dot,"pc":s.cpu.pc,"sr":s.cpu.sr.0,"dma":s.vdp.dma_bus_stall_master_cycles}))?;
            s.step_instruction(); master += 44*7; steps += 1; continue;
        }
        let pc = s.cpu.pc;
        if ordinal < trace_limit {for v in [pc, s.cpu.sr.0 as u32].into_iter().chain(s.cpu.d).chain(s.cpu.a) { out.write_all(&v.to_le_bytes())?; }}
        let frame=s.vdp.frame; let line=s.vdp.line; let dot=s.vdp.dot;
        let dma=s.vdp.dma_bus_stall_master_cycles;
        let regs=s.vdp.registers; let scroll=s.vdp.vsram[..4].to_vec();
        let data_count=s.vdp_data_counts.iter().sum::<u64>();
        let d=s.cpu.d; let a=s.cpu.a;
        if refresh && master >= refresh_at {
            refresh_at = master + 896;
            master += 14; s.vdp.advance_master_cycles(14); refresh_delays += 1;
        }
        let (cycles, ex)=s.step_instruction();
        let dw=s.vdp.dma_bus_stall_master_cycles-dma;
        if fifo && regs[1]&0x40 != 0 && !(224..261).contains(&line) {
            let slots:&[u64]=if regs[12]&1 != 0 {&[352,820,948,1076,1332,1460,1588,1844,1972,2100,2356,2484,2612,2868,2996,3124,3364,3380]} else {&[230,510,810,970,1130,1450,1610,1770,2090,2250,2410,2730,2890,3050,3350,3370]};
            let base=master/3420*3420;
            for _ in data_count..s.vdp_data_counts.iter().sum::<u64>() {
                let mut ready=master;
                if ready < fifo_cycles[(fifo_idx+3)&3] {
                    if ready < fifo_cycles[fifo_idx] {
                        let wait=fifo_cycles[fifo_idx].div_ceil(7)*7-master;
                        fifo_waits+=wait;master+=wait;s.vdp.advance_master_cycles(wait as u32);
                    }
                    ready=fifo_cycles[(fifo_idx+3)&3];
                }
                let mut candidates=Vec::new();
                for l in 0..4 {for x in slots {candidates.push(base+l*3420+x);}}
                let slot=candidates.iter().position(|x|*x>ready).unwrap();
                let byte_access=usize::from(s.vdp.code&6==0);
                fifo_cycles[fifo_idx]=candidates[slot+byte_access];fifo_idx=(fifo_idx+1)&3;
            }
        }
        master += u64::from(cycles)*7+dw-dw.div_ceil(7)*7;
        if ordinal < trace_limit {for v in [ordinal as u32,frame as u32,line,dot,cycles,dw as u32] { timing.write_all(&v.to_le_bytes())?; }}
        if ex.is_some() { return Err(format!("exception at {pc:x}").into()); }
        if pc==0x16e0 || regs != s.vdp.registers || scroll != s.vdp.vsram[..4] || dma!=s.vdp.dma_bus_stall_master_cycles {
            writeln!(events,"{}",serde_json::json!({"kind":"vdp","ordinal":ordinal,"frame":frame,"line":line,"dot":dot,"pc":pc,"d":d,"a":a,"cycles":cycles,"dma_delta":dw,"registers":s.vdp.registers.to_vec(),"scroll":s.vdp.vsram[..4].to_vec()}))?;
        }
        ordinal+=1; steps+=1;
        if frame != s.vdp.frame {fifo_cycles=[0;4];if vint_delay {deferred_vint=true;}}
    }
    out.flush()?; timing.flush()?; events.flush()?;
    let (fb,_) = s.vdp.render();
    std::fs::write(format!("{}.rgb",args[3]),fb.data)?;
    std::fs::write(format!("{}.vsram",args[3]),&s.vdp.vsram)?;
    println!("{}",serde_json::json!({"rom_sha256":s.cart.sha256,"records":ordinal,"steps":steps,"frame":s.vdp.frame,"pc":s.cpu.pc,"dma":s.vdp.dma_bus_stall_master_cycles,"refresh_delays":refresh_delays,"fifo_waits_master":fifo_waits,"master":master,"scroll":s.vdp.vsram[..4].to_vec(),"layout_timing":"6 little-endian u32: ordinal,frame,line,dot,step CPU cycles incl ceil DMA, DMA master-cycle delta"}));
    Ok(())
}
