use mdcore::{Bus,M68000,SparseBus};
use mdcore::cpu::{disasm,m68000::Exception};

#[test]
fn both_usp_directions_are_valid_two_byte_privileged_instructions() {
    for register in 0..8 {
        for from_usp in [false,true] {
            let opcode:u16=0x4e60|register|if from_usp {8} else {0};
            let instruction=disasm::disassemble(&opcode.to_be_bytes(),0x200);
            assert!(instruction.decoded);assert_eq!(instruction.length,2);
            assert_eq!(instruction.text,if from_usp {format!("move.l usp,a{register}")} else {format!("move.l a{register},usp")});
            let mut bus=SparseBus::new();bus.write16(0x200,opcode);
            let mut cpu=M68000::new(0x200);cpu.usp=0x12345678;cpu.a[register as usize]=0x87654321;cpu.sr.0=0x2700;cpu.prime(&mut bus);
            assert_eq!(cpu.step(&mut bus).exception,None);
            if from_usp {assert_eq!(cpu.a[register as usize],0x12345678);} else {assert_eq!(cpu.usp,0x87654321);}
            let mut cpu=M68000::new(0x200);cpu.usp=0x12345678;cpu.a[register as usize]=0x87654321;cpu.sr.0=0x0700;cpu.prime(&mut bus);
            assert_eq!(cpu.step(&mut bus).exception,Some(Exception::Privileged));
            assert_eq!(cpu.usp,0x12345678);assert_eq!(cpu.a[register as usize],0x87654321);assert_eq!(cpu.pc,0x200);
        }
    }
}

#[test]
fn polling_loop_nops_are_decoded_as_supported_two_byte_instructions() {
    let instruction=disasm::disassemble(&[0x4e,0x71],0x740);
    assert!(instruction.decoded);assert_eq!(instruction.length,2);assert_eq!(instruction.text,"nop");
}
