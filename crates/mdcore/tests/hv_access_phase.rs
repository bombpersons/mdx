//! Literal values come from96 reads executed by the unchanged GPGX core.
use mdcore::{Cartridge,System};

#[test]
fn counter_sampling_matches_independent_programs_including_refresh_boundaries() {
    let reference=include_bytes!("fixtures/hv_read_samples.bin");
    for (profile,(mode5,wide)) in [(false,false),(true,false),(true,true)].into_iter().enumerate() {
        let mut rom=vec![0;65536];rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        let mut code:Vec<u16>=Vec::new();
        if mode5 {code.extend([0x33fc,0x8104,0x00c0,0x0004]);}
        if wide {code.extend([0x33fc,0x8c81,0x00c0,0x0004]);}
        for sample in 0..32 {
            code.extend([0x3039,0x00c0,0x0008,0x33c0,0x00ff,0xf000+sample*2]);
            code.extend([0x4e71;13]);
        }
        code.push(0x60fe);
        for (i,word) in code.iter().enumerate() {rom[0x200+i*2..0x202+i*2].copy_from_slice(&word.to_be_bytes());}
        let mut system=System::new(Cartridge::from_bytes("phase.md",rom).unwrap());
        let mut steps=0;
        while system.vdp.frame<1 && steps<1000000 {assert_eq!(system.step_instruction().1,None);steps+=1;}
        assert_eq!(system.vdp.frame,1);
        assert_eq!(&system.wram[0xf000..0xf040],&reference[profile*64..profile*64+64],"profile={profile}");
    }
}
