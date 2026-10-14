//! Independent 68000 JSR/JMP timings from GPGX's m68kops.h opcode table.
use mdcore::{Bus, SparseBus, M68000};

#[test]
fn control_transfers_match_all_legal_mode_costs_targets_and_return_addresses() {
    for (opcode, cycles) in [
        (0x4e90, 16),
        (0x4ea8, 18),
        (0x4eb0, 22),
        (0x4eb8, 18),
        (0x4eb9, 20),
        (0x4eba, 18),
        (0x4ebb, 22),
        (0x4ed0, 8),
        (0x4ee8, 10),
        (0x4ef0, 14),
        (0x4ef8, 10),
        (0x4ef9, 12),
        (0x4efa, 10),
        (0x4efb, 14),
    ] {
        let mut bus = SparseBus::new();
        let mut cpu = M68000::new(0x200);
        cpu.a[7] = 0xff8000;
        cpu.d[1] = 4;
        bus.write16(0x200, opcode);
        let next_pc = match ((opcode >> 3) & 7, opcode & 7) {
            (2, _) => {
                cpu.a[0] = 0x250;
                0x202
            }
            (5, _) => {
                cpu.a[0] = 0x248;
                bus.write16(0x202, 8);
                0x204
            }
            (6, _) => {
                cpu.a[0] = 0x248;
                bus.write16(0x202, 0x1004);
                0x204
            }
            (7, 0) => {
                bus.write16(0x202, 0x250);
                0x204
            }
            (7, 1) => {
                bus.write32(0x202, 0x250);
                0x206
            }
            (7, 2) => {
                bus.write16(0x202, 0x4e);
                0x204
            }
            (7, 3) => {
                bus.write16(0x202, 0x104a);
                0x204
            }
            _ => unreachable!(),
        };
        bus.write16(0x250, 0x4e75); // RTS
        bus.write16(0x252, 0x4e71);
        cpu.prime(&mut bus);
        let before_sr = cpu.sr;
        let outcome = cpu.step(&mut bus);
        assert_eq!(outcome.exception, None, "opcode={opcode:04x}");
        assert_eq!(outcome.cycles, cycles, "opcode={opcode:04x}");
        assert_eq!(cpu.pc, 0x250, "opcode={opcode:04x}");
        assert_eq!(cpu.prefetch, [0x4e75, 0x4e71]);
        assert_eq!(cpu.sr, before_sr);
        if opcode & 0x40 == 0 {
            assert_eq!(cpu.a[7], 0xff7ffc);
            assert_eq!(bus.read32(cpu.a[7]), next_pc, "opcode={opcode:04x}");
            assert_eq!(cpu.step(&mut bus).exception, None);
            assert_eq!(cpu.pc, next_pc);
        }
        assert_eq!(cpu.a[7], 0xff8000);
    }
}

#[test]
fn absolute_short_control_targets_are_sign_extended() {
    for (opcode, cycles) in [(0x4eb8, 18), (0x4ef8, 10)] {
        let mut bus = SparseBus::new();
        bus.write16(0x200, opcode);
        bus.write16(0x202, 0x8000);
        bus.write16(0xffff8000, 0x4e71);
        let mut cpu = M68000::new(0x200);
        cpu.a[7] = 0xff7000;
        cpu.prime(&mut bus);
        let outcome = cpu.step(&mut bus);
        assert_eq!(outcome.exception, None);
        assert_eq!(outcome.cycles, cycles);
        assert_eq!(cpu.pc, 0xffff8000);
        assert_eq!(cpu.prefetch[0], 0x4e71);
    }
}
