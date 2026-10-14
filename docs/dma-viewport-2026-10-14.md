# DMA across the initial viewport latch

Both previously failing large active-VRAM transfers now match their unchanged
independent samples. Their portable regression is enabled in the default suite.
An expanded96-case matrix also matches all3072 HV samples: H32/H40,224/240-line
viewports, VRAM/CRAM/VSRAM,16/4096/8192/16384 words and two startup phases.

The wait calculator used the viewport height at DMA start for every future frame.
Reset begins with192 visible lines; software can enable Mode5 while that initial
frame is still running. The actual VDP latches the new height on the frame edge,
changing the blank interval and DMA capacity. The predictive transfer loop now
latches the same height on that edge and moves its local line to the new viewport
boundary. The rate equations and frozen expected samples were preserved.

The correction changes a prediction using existing captured VDP state; the machine
save-state layout remains format3. Cadash2700→4500 full-state, memory, image and
loaded boot/trace/watch parity still pass. All27 frozen image checkpoints pass.
The workspace now passes216 default Rust tests (23 ignored), including the new
standalone PSG checks; six Python observer/capture tests pass.

[Portable evidence](evidence/dma-viewport-session.json) retains source/reference
hashes and validation scope. Local runs are under ignored `.mdx/dma-frame-transition/`.
No commercial assets or downloaded emulator sources were added to Git. Original
oracle execution, replay inputs and expected captures remain unchanged. Internal
fill/copy timing, arbitrary register changes during internal DMA and full hardware
equivalence are not established by this external-transfer checkpoint.

The next work is the queued sound feature. See [audio-development.md](audio-development.md)
for the tested PSG component, independent reference WAV capture and FM dependency
fetch required before synthesis can be integrated into the tool.
