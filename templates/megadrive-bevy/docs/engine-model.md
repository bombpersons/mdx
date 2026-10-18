# Engine model

What the original treats as **data** (the engine interprets it) and what as
**code** (per-type controllers in `game/`), worked out from evidence before
writing gameplay code. Keep it current: it defines the engine's scope.
For each answer, cite the evidence (addresses, tables, traces, mdx runs).

## World

What is a level made of (tiles grouped into blocks and chunks, a layout of
chunk ids, separate foreground and background planes)? How is collision
stored (per-tile solidity and height or angle arrays, or a separate map)?
How does the camera scroll and how are planes and the window plane used?

_Not yet established._

## Movement

How do objects move: fixed-point positions and speeds (16.16, 16.8), with
which subpixel rounding? Gravity, slopes, acceleration as code or tables?

_Not yet established._

## Objects

How are objects defined: object slots in RAM, a type id per slot and a
pointer table of per-type routines? A routine counter for state? Which
fields come from level placement data and which are hardcoded?

_Not yet established._

## Level scripting

How do levels script events: event tables keyed on camera position, special
objects, routines hardcoded per level?

_Not yet established._

## Frame

How is a frame run: what the vertical blank handler does (pad read, DMA of
sprites and palettes) and what the main loop does, in which order; does the
game wait one or two frames per update (`FRAMES_PER_TICK`); what happens on
lag frames? `engine::schedule` mirrors the answer.

_Not yet established._

## Sound

Which sound driver (Z80 or 68000, which family), how the game requests
music and effects, where the sequence data lives. The driver is engine;
songs and effects are data.

_Not yet established._
