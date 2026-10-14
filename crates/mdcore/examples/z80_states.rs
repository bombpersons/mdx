fn main() {
    for bytes in [[0xCB, 0x46, 0, 0], [0x3A, 0x00, 0x40, 0], [0x36, 0x27, 0, 0], [0x7E, 0, 0, 0], [0xDD, 0x7E, 0x00, 0]] {
        println!("{:02X?} -> {}", bytes, mdcore::z80::instruction_states(bytes));
    }
}
