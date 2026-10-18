# md-bevy: edit sparingly, and record every edit

This crate belongs to the mdx template. `mdx template update` replaces the
files you haven't touched with newer versions. A file you edited is left as
it is, and the new version lands beside it as `<file>.mdx-new` for you to
merge. Edits are allowed; the aim is that updates stay easy to merge.

First try building on it without editing:

- New rendering, input or test helpers for this engine family: add them to
  `engine/`, wrapping or re-exporting what is here.
- Game-specific behaviour: `game/`.
- A launcher flag or setup step the game needs: add plugins in the `add`
  closure of `launcher::Game`, or build the app with `launcher::app` and
  adjust it before `run()`.
- A visual effect the look doesn't have (line scroll, the window plane,
  shadow/highlight, raster colour changes): build it in `engine/` from
  planes, sprites and palettes, as data the game sets each tick.

Edit here when that can't be done cleanly, e.g. a missing hook or launcher
setting:

- Only general Mega Drive machinery. Nothing specific to this game: that
  goes in `game/`, or in `engine/` behind a hook the game fills.
- Never turn the look into an emulator: no VRAM, nametable addresses, VDP
  registers, DMA or CRAM write timing. Effects are data (tiles, maps,
  palettes, scroll values, draw order), not register writes.
- Keep edits small and local. Add rather than rewrite; don't move, rename
  or reformat code; keep the existing public API working.
- List every edited file in `docs/template.md` with the change and why.
- On update, merge each `<file>.mdx-new` into your file (take the new
  version and reapply the change listed in `docs/template.md`), then delete
  the `.mdx-new`.
- If the edit would help any game, note it for the mdx toolkit as well, so
  it can arrive upstream and the local edit can be dropped.
