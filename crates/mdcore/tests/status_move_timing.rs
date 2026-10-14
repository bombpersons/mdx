//! Literal clock counts from the unchanged GPGX 68000 opcode table in
//! core/m68k/m68kops.h (move_16_frs/toc/tos). Exercise every legal EA class.
use mdcore::{Bus, SparseBus, M68000};

#[test]
fn status_moves_match_independent_addressing_mode_costs_and_values() {
    let cases = [
        (0x40c0, 6),
        (0x40d0, 12),
        (0x40d8, 12),
        (0x40e0, 14),
        (0x40e8, 16),
        (0x40f0, 18),
        (0x40f8, 16),
        (0x40f9, 20),
        (0x44c0, 12),
        (0x44d0, 16),
        (0x44d8, 16),
        (0x44e0, 18),
        (0x44e8, 20),
        (0x44f0, 22),
        (0x44f8, 20),
        (0x44f9, 24),
        (0x44fa, 20),
        (0x44fb, 22),
        (0x44fc, 16),
        (0x46c0, 12),
        (0x46d0, 16),
        (0x46d8, 16),
        (0x46e0, 18),
        (0x46e8, 20),
        (0x46f0, 22),
        (0x46f8, 20),
        (0x46f9, 24),
        (0x46fa, 20),
        (0x46fb, 22),
        (0x46fc, 16),
    ];
    for (opcode, expected) in cases {
        let mut bus = SparseBus::new();
        let mut cpu = M68000::new(0x200);
        cpu.sr.0 = 0x2700;
        cpu.d[0] = 0x2005;
        cpu.a[0] = 0x1000;
        bus.write16(0x200, opcode);
        let mode = (opcode >> 3) & 7;
        let reg = opcode & 7;
        let address = match (mode, reg) {
            (4, _) => 0x0ffe,
            (5 | 6, _) => {
                bus.write16(0x202, if mode == 6 { 0x1008 } else { 8 });
                0x1008
            }
            (7, 0) => {
                bus.write16(0x202, 0x1000);
                0x1000
            }
            (7, 1) => {
                bus.write32(0x202, 0x1000);
                0x1000
            }
            (7, 2) => {
                bus.write16(0x202, 0x0dfe);
                0x1000
            }
            (7, 3) => {
                bus.write16(0x202, 0x1008);
                0x20a
            }
            (7, 4) => {
                bus.write16(0x202, 0x2005);
                0x1000
            }
            _ => 0x1000,
        };
        bus.write16(address, 0x2005);
        cpu.prime(&mut bus);
        let outcome = cpu.step(&mut bus);
        assert_eq!(outcome.exception, None, "opcode={opcode:04x}");
        assert_eq!(outcome.cycles, expected, "opcode={opcode:04x}");
        match opcode & 0xffc0 {
            0x40c0 => {
                assert_eq!(cpu.sr.0, 0x2700);
                let value = if mode == 0 {
                    cpu.d[0] as u16
                } else {
                    bus.read16(address)
                };
                assert_eq!(value, 0x2700, "opcode={opcode:04x}");
            }
            0x44c0 => assert_eq!(cpu.sr.0, 0x2705, "opcode={opcode:04x}"),
            0x46c0 => assert_eq!(cpu.sr.0, 0x2005, "opcode={opcode:04x}"),
            _ => unreachable!(),
        }
    }
}
