//! Versioned instruction-boundary machine states. ROM bytes and observers are excluded.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineState {
    format: String,
    version: u32,
    engine_version: String,
    rom_sha256: String,
    rom_bytes: usize,
    wram: Vec<u8>,
    vdp: Vdp,
    io: Io,
    cpu: M68000,
    z80_ram: Vec<u8>,
    z80: Z80,
    fm: Fm,
    audio: crate::audio::AudioState,
    pending_dma_master_cycles: u32,
    dma_overlaps_instruction: bool,
    pending_bus_cpu_cycles: u32,
    z80_master_clock: u64,
    z80_dma_until: u64,
    pending_z80_cpu_wait: u32,
    sound_prefetch_word: u16,
    master_cycles: u64,
    refresh_deadline: u64,
    pending_irq_ack: Option<(u8, u64)>,
    pending_fifo_master_cycles: u32,
    /// Absent in states saved before cartridge hardware; restored at power-on values.
    #[serde(default)]
    cart_hw: Option<crate::cart_hw::CartHw>,
}

impl System {
    /// Capture between completed system steps; observers start fresh on restore.
    pub fn save_state(&self) -> Result<MachineState, String> {
        if self.cpu_bus_active
            || self.z80_bus_active
            || self.interrupt_bus_active
            || self.deferred_z80_sync.is_some()
            || !self.audio_step_events.is_empty()
        {
            return Err("machine states require an instruction boundary".into());
        }
        Ok(MachineState {
            format: "mdx.machine-state".into(),
            version: 4,
            engine_version: crate::MDCORE_VERSION.into(),
            rom_sha256: self.cart.sha256.clone(),
            rom_bytes: self.cart.data.len(),
            wram: self.wram.clone(),
            vdp: self.vdp.clone(),
            io: self.io.clone(),
            cpu: self.cpu.clone(),
            z80_ram: self.z80_ram.clone(),
            z80: self.z80,
            fm: self.fm.clone(),
            audio: self.audio.save_state(),
            pending_dma_master_cycles: self.pending_dma_master_cycles,
            dma_overlaps_instruction: self.dma_overlaps_instruction,
            pending_bus_cpu_cycles: self.pending_bus_cpu_cycles,
            z80_master_clock: self.z80_master_clock,
            z80_dma_until: self.z80_dma_until,
            pending_z80_cpu_wait: self.pending_z80_cpu_wait,
            sound_prefetch_word: self.sound_prefetch_word,
            master_cycles: self.master_cycles,
            refresh_deadline: self.refresh_deadline,
            pending_irq_ack: self.pending_irq_ack,
            pending_fifo_master_cycles: self.pending_fifo_master_cycles,
            cart_hw: Some(self.cart_hw.clone()),
        })
    }

    /// Restore all modeled hardware without resetting or re-fetching the CPU queue.
    pub fn from_state(cart: Cartridge, state: MachineState) -> Result<Self, String> {
        if state.format != "mdx.machine-state"
            || state.version != 4
            || state.engine_version != crate::MDCORE_VERSION
        {
            return Err("incompatible machine state format/version".into());
        }
        if state.rom_sha256 != cart.sha256 || state.rom_bytes != cart.data.len() {
            return Err("machine state belongs to a different ROM".into());
        }
        // Bound imported clocks away from arithmetic overflow, and validate indexes
        // before any instruction or device access can use the imported state.
        if state.wram.len() != WRAM_SIZE
            || state.z80_ram.len() != Z80_RAM_SIZE
            || !state.vdp.valid_state(state.master_cycles)
            || !state.fm.valid_state()
            || state.z80.im > 2
            || state.master_cycles > u64::MAX / 4096
            || state.refresh_deadline > u64::MAX / 4096
            || state.pending_dma_master_cycles > 100_000_000
            || state.pending_fifo_master_cycles > 100_000_000
            || state.pending_bus_cpu_cycles > 100_000_000
            || state
                .cpu
                .refresh_block_cycles
                .is_some_and(|n| n > 100_000_000)
            || state.z80.cycles > u64::MAX / 4096
            || state.z80_master_clock > u64::MAX / 4096
            || state.z80_dma_until > u64::MAX / 4096
            || state.pending_z80_cpu_wait > 100_000_000
            || state
                .pending_irq_ack
                .is_some_and(|(level, at)| !(1..=7).contains(&level) || at > state.master_cycles)
            || state.io.zbank & !0xFF8000 != 0
            || state.cart_hw.as_ref().is_some_and(|hw| !hw.valid())
        {
            return Err("invalid machine state memory, device indexes or clock state".into());
        }
        let audio = crate::audio::Audio::from_state(state.audio, state.master_cycles)?;
        let mut system = Self::new(cart);
        system.audio = audio;
        system.wram = state.wram;
        system.vdp = state.vdp;
        system.io = state.io;
        system.cpu = state.cpu;
        system.z80_ram = state.z80_ram;
        system.z80 = state.z80;
        system.fm = state.fm;
        system.fm_offset = state.master_cycles.wrapping_sub(system.fm.clock());
        system.pending_dma_master_cycles = state.pending_dma_master_cycles;
        system.dma_overlaps_instruction = state.dma_overlaps_instruction;
        system.pending_bus_cpu_cycles = state.pending_bus_cpu_cycles;
        system.z80_master_clock = state.z80_master_clock;
        system.z80_dma_until = state.z80_dma_until;
        system.pending_z80_cpu_wait = state.pending_z80_cpu_wait;
        system.sound_prefetch_word = state.sound_prefetch_word;
        system.master_cycles = state.master_cycles;
        system.refresh_deadline = state.refresh_deadline;
        system.pending_irq_ack = state.pending_irq_ack;
        system.pending_fifo_master_cycles = state.pending_fifo_master_cycles;
        if let Some(cart_hw) = state.cart_hw {
            system.cart_hw = cart_hw;
        }
        Ok(system)
    }
}

pub(crate) mod array {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer, T: Serialize, const N: usize>(
        value: &[T; N],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_slice().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
        d: D,
    ) -> Result<[T; N], D::Error> {
        Vec::<T>::deserialize(d)?.try_into().map_err(|v: Vec<T>| {
            serde::de::Error::custom(format!("expected {N} array entries, got {}", v.len()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded(system: &System) -> Vec<u8> {
        serde_json::to_vec(&system.save_state().unwrap()).unwrap()
    }
    #[test]
    fn resumes_hidden_device_phase_and_cpu_prefetch_exactly() {
        let cart = crate::test_support::cartridge();
        let mut live = System::new(cart.clone());
        // Exercise independently running sound CPU, YM timers, controllers, VDP
        // command/fill latches and FIFO at a non-frame-aligned boundary.
        live.io.z80_reset = false;
        live.z80_ram[..6].copy_from_slice(&[0x21, 0x00, 0x01, 0x34, 0x18, 0xfd]);
        live.io.pad_buttons = [0x15, 0x22];
        live.fm.write(0, 0x24);
        live.fm.write(1, 0xff);
        live.fm.write(0, 0x25);
        live.fm.write(1, 3);
        live.fm.write(0, 0x27);
        live.fm.write(1, 5);
        for _ in 0..137 {
            live.step_instruction();
        }
        live.vdp.control_write(1, 0x44);
        live.vdp.address_latch = 0x1234;
        live.vdp.control_pending = true;
        live.vdp.start_internal_dma(4000, false);
        live.vdp.fifo_write_wait(0);
        live.cpu.override_branch(0x202, true);
        // Preserve a stale queue rather than rebuilding it from cartridge bytes.
        live.cpu.prefetch = [0x4e71, 0x60fc];
        live.pending_irq_ack = Some((6, live.master_cycles - 17));
        let bytes = encoded(&live);
        let state: MachineState = serde_json::from_slice(&bytes).unwrap();
        let mut resumed = System::from_state(cart, state).unwrap();
        assert_eq!(bytes, encoded(&resumed));
        for _ in 0..50_000 {
            assert_eq!(live.step_instruction(), resumed.step_instruction());
        }
        assert_eq!(encoded(&live), encoded(&resumed));
        assert!(live.z80_ram[0x100] != 0);
        assert_eq!(live.vdp.frame().0.data, resumed.vdp.frame().0.data);
    }
    #[test]
    fn rejects_wrong_rom_versions_and_malformed_devices() {
        let cart = crate::test_support::cartridge();
        let original =
            serde_json::to_value(System::new(cart.clone()).save_state().unwrap()).unwrap();
        for (pointer, value) in [
            ("/version", serde_json::json!(1)),
            ("/rom_sha256", serde_json::json!("wrong")),
            ("/wram", serde_json::json!([])),
            ("/vdp/fifo_index", serde_json::json!(4)),
            ("/vdp/dot", serde_json::json!(896)),
            ("/vdp/elapsed_scaled", serde_json::json!(0)),
            ("/fm/address", serde_json::json!(512)),
            ("/z80/im", serde_json::json!(3)),
            ("/fm/timer_samples", serde_json::json!(999999)),
            ("/vdp/frame", serde_json::json!(u64::MAX)),
        ] {
            let mut bad = original.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            let state = serde_json::from_value(bad).unwrap();
            assert!(
                System::from_state(cart.clone(), state).is_err(),
                "{pointer}"
            );
        }
        let mut bad = original;
        bad["fm"]["registers"] = serde_json::json!([1, 2]);
        assert!(serde_json::from_value::<MachineState>(bad).is_err());
    }
}
