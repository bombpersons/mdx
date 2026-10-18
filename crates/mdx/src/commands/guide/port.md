A port is built as two things from the start: an engine (the reusable machinery
the game runs on, usable for a different game without the original's content
or rules) and this game on top of it. Matching the original and reusing the
engine don't conflict when the engine is cut where the original cuts: data the
original interprets from tables becomes engine formats; code the original runs
per object type stays code, ported faithfully. Invented abstractions (a generic
enemy schema, an event scripting language, a physics component replacing
hand-written movement) lose update order, fixed-point rounding and special
cases. Restructuring a finished port is much harder, so set the structure up
first.

It is not an emulator. No 68000 or Z80 interpreter, VRAM or nametable buffers,
VDP register writes, DMA queues or memory maps in the port. Tile uploads
become tile maps and art, colour RAM writes become palette edits, object
slots become entities, routine pointer tables become registered types.

Evidence: the ROM and mdx are the only sources. Back every ported behaviour
with runtime observations or binary evidence and say which in
docs/evidence.md.

Work order:
1. Boot the game (boot with frames and input) and save reference states
   (save_state) at the moments you will study: title, each level start,
   representative gameplay.
2. Write docs/engine-model.md from evidence before gameplay code: what a level
   is made of (tiles, blocks, chunks, layouts; how collision is stored); how
   objects are defined (slot table in RAM, type id, per-type routine pointer
   table, routine counter); how positions and speeds are stored (16.16, 16.8);
   how levels script events; how a frame runs (vertical blank handler versus
   main loop, frames per update, lag frames); which sound driver plays what.
   disasm, trace (instruction records) and watch (exact writer of a RAM
   address) answer these.
3. template_create into the game folder (it keeps the ROM and an existing
   AGENTS.md). The workspace is md-bevy/ (template-owned; small general edits
   allowed, listed in docs/template.md), engine/ (registries, update-order
   sets, level and tuning formats, a sandbox example), game/ (demo to replace)
   and tools/import/. Reshape the engine skeleton to match engine-model.md;
   keep the sandbox building.
4. Importers in tools/import read the ROM at the offsets you found and write
   engine formats (indexed PNG tiles and sprites, RON maps, palettes and
   tables, WAV) under game/assets, with provenance. Validate structurally:
   every source byte accounted for, every reference in range, decompressed
   art identical to what graphics shows in VRAM after the game loads it.
   Raw formats live only in importers.
5. Port feature by feature: record the original (boot from a saved state with
   an input replay and samples of the RAM the feature's state lives in), place
   the code by the rules in engine/AGENTS.md, add a trajectory test against
   the samples (exact, tolerance 0) or a screenshot check (image.compare of an
   mdx screenshot and the port's game_screenshot), run the sandbox.
6. Keep docs current: what is verified, against what, what is provisional.

Placement, in short: needed by another game on this engine -> engine/, named
generically; read from a table in the original -> data the engine interprets;
hardcoded per type -> a registered controller in game/; a design number inside
a controller -> its tuning file. Never in engine/: this game's names, asset
paths, ROM addresses, table magic ids or checks for which game is running.
Unknown type ids and actions are reported, never mapped to something else.

The sandbox (cargo run -p engine --example sandbox) is a tiny second game made
only from engine/ and hand-written assets. If a feature can't be used there
without reaching into game/, move the boundary, not the sandbox.

The look: md-bevy draws with the console's conventions as ordinary Bevy
rendering: 9-bit colours with mdx's exact level mapping, four 16-colour
palette lines, indexed art, scrolling tile planes with per-tile priority, and
the original's draw order. Effects it lacks (line scroll, the window plane,
shadow/highlight) are built in engine/ as data the game sets each tick.

Keep an editor growing with the port: once a format exists in the engine, the
editor can open, show and save it, through engine/ alone (registry metadata
lists the game's types). Grow it a format at a time, after the format has
settled and the matching trajectory tests pass. Saving an unedited level
reproduces the file exactly. An editor that needs game/ is a boundary bug.

The editor is a modern desktop application, not part of the game's look. It
runs at the window's full resolution, is driven by mouse and keyboard, and
uses ordinary UI (panels, menus, text fields, drag and drop; egui or Bevy UI
at native scale). Don't build it on md-bevy's low-resolution screen, controls
menu or bitmap font. Its viewports may render levels and sprites, but the tool
itself should look like a current level editor, not a Mega Drive game.

Fidelity is the contract: every restructuring keeps every trajectory test
passing exactly. If a refactor changes a number, find out from evidence
whether the refactor is wrong or the test relied on an accident. Compare
rendered frames against the original and look at them.

Recording a trajectory (boot): from_state is the saved state, input a
controller replay ({"events":[{"frame":0,"buttons":["right"]}, ...]}), frames
how long, and samples one entry per value, e.g.
{"address":"0xFFD008","type":"s16","label":"x"} (types u8 s8 u16 s16 u32 s32
fx8.8 fx16.16). The run writes a CSV (data.samples.artifact): row N is memory at the start of
vertical blank of run frame N, before that frame's input; row 0 is the
starting state. md-bevy's testing::trajectory reads it.

Template updates: template_update replaces untouched md-bevy/ files, writes
<file>.mdx-new beside edited ones (reapply the edit listed in
docs/template.md) and copies changed seeded files under
.mdx/template-update/ for you to merge. dry_run reports first.

Done means: the port matches recorded behaviour and looks like the original;
a new game could add a crate like game/ and author engine-format assets
without editing engine/; levels, object setup and tuning change by editing
assets, in files or in the editor.
