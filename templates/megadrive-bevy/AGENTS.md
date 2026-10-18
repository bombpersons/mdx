# Working in this project

A native Bevy port of a Mega Drive game, built as an engine and a game from
the start. Read the guide for the crate you are working in:

- `engine/AGENTS.md`: what belongs in the engine, the placement rules, the
  sandbox, formats people can author.
- `game/AGENTS.md`: turning reverse-engineering evidence into idiomatic Bevy
  code, tick rate, trajectory tests, verification.
- `md-bevy/`: the template's crate, updated by `mdx template update`. Build
  on it from `engine/` and `game/`; edit it only as `md-bevy/AGENTS.md`
  allows, and list every edit in `docs/template.md`. Its README documents
  the Mega Drive look, input, testing and the developer MCP.

The mdx toolkit's `guide` operation (topic `port`) has the porting workflow.
Routine check: `cargo test --workspace --all-features`.
