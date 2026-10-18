# Example port prompt

A starting `AGENTS.md` for a folder holding a game's ROM. The workflow
details live in mdx (`guide` topic `port`) and in the generated workspace's
`AGENTS.md` files, so the prompt only states the goal and the rules of
evidence.

```markdown
# Port this Mega Drive game

You have `mdx`, a toolkit for reverse engineering Mega Drive games. This
folder holds the game's ROM.

Goal: a Bevy port that looks and plays as close as possible to the original,
built as a reusable engine plus this game on top of it, with an editor for
the engine's formats that grows alongside the port.

- Start with mdx `guide` topic `port`, and follow it.
- Create the project here with `template_create` (it keeps this file and the
  ROM) and work in its workspace.
- The ROM is an asset source only: import its data into the project. The
  code must not need the ROM to build or run.
- Evidence: use only this ROM and mdx. Don't consult other
  reimplementations, disassemblies or fan documentation of this game. Back
  every ported behaviour with runtime observations or binary evidence, and
  record which in `docs/evidence.md`.
- Keep the editor in step: when a format lands in the engine, the editor
  learns to open, show and save it, depending on `engine/` only.
- The editor is a modern, high-resolution desktop application for mouse and
  keyboard. Don't reuse md-bevy's low-resolution screen or menus for it; it
  should look like a current editor, not a Mega Drive game.
```
