//! Numeric regression of condition encodings against Z80/GPGX semantics.
use mdcore::z80::{Z80,Z80Bus};
struct Memory(Vec<u8>);
impl Z80Bus for Memory {
    fn read(&mut self,a:u16)->u8 {self.0[a as usize]}
    fn write(&mut self,a:u16,v:u8) {self.0[a as usize]=v;}
}
fn main() {
    let mut failures=Vec::new();let mut cases=0;
    for bits in 0..16u8 {
        let f=(if bits&1!=0 {0x40} else {0})|(if bits&2!=0 {1} else {0})|(if bits&4!=0 {4} else {0})|(if bits&8!=0 {0x80} else {0});
        let expected=[bits&1==0,bits&1!=0,bits&2==0,bits&2!=0,bits&4==0,bits&4!=0,bits&8==0,bits&8!=0];
        for cc in 0..8usize {
            let mut cpu=Z80::new();cpu.pc=0x1000;cpu.f=f;
            let mut ram=Memory(vec![0;65536]);ram.0[0x1000]=0xc2+cc as u8*8;ram.0[0x1001]=0;ram.0[0x1002]=0x20;
            cpu.step(&mut ram);cases+=1;
            let want=if expected[cc] {0x2000} else {0x1003};
            if cpu.pc!=want {failures.push(serde_json::json!({"op":"JP cc","cc":cc,"f":f,"actual_pc":cpu.pc,"expected_pc":want}));}
        }
    }
    // Literal observed driver: AND A for command $0D leaves Z=0,S=0.
    let mut cpu=Z80::new();cpu.pc=0x1a1;cpu.a=13;cpu.f=0x18;
    let mut ram=Memory(vec![0;65536]);ram.0[0x1a1]=0x28;ram.0[0x1a2]=0x0e;
    cpu.step(&mut ram);cases+=1;
    if cpu.pc!=0x1a3 {failures.push(serde_json::json!({"op":"driver JR Z","actual_pc":cpu.pc,"expected_pc":0x1a3}));}
    println!("{}",serde_json::json!({"cases":cases,"failed":failures.len(),"failures":failures}));
}
