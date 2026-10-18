mdx runs Mega Drive cartridges in an accurate emulator built for looking
inside them. Every operation returns JSON that says what it observed, what it
left out (coverage) and where its files are. Large results are artifacts:
read them with artifact_view (JSON pointers, record pages) or artifact_read
(bytes).

Pass project_root (your workspace) on every MCP call; relative paths and the
.mdx/artifacts store resolve there.

A typical investigation:
1. header: what the cartridge declares (region, ROM/RAM ranges, vectors).
2. boot with frames (and an input replay to press buttons): screenshot,
   memory dumps, VDP state. save_state publishes a state; from_state resumes
   one, so later runs start where the interesting part begins.
3. Find the code: watch reports the exact instruction that writes a RAM
   address; trace records executed instructions; disasm decodes a range;
   memory_search and memory_diff find values and what changed between two
   snapshots.
4. Find the data: graphics exports patterns, palettes, name tables and
   sprites from a snapshot; render redraws it; audio and the VGM/stem exports
   capture sound.
5. Compare: image_compare diffs two PNGs (an mdx screenshot and a port's).

Results are evidence about the runs you made, not proofs about the whole
game. Write down which run supports which conclusion.

Porting a game to native code: read guide topic port before starting. It
sets up a workspace (template_create) that keeps a strict boundary between a
reusable engine and the game, and explains how to record trajectories (boot
with samples) to test the port against the original.
