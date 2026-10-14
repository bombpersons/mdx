> Follow-up: the user installed the Codex registration. Per-call project/store
> selection and the updated OpenCode/Codex installers are documented in
> [project-context.md](project-context.md). Older launch-root details below are
> the historical reorganisation checkpoint.

# Workspace reorganisation — 2026-10-13

`mdx/` owns the reusable Rust toolkit and all its development/validation work.
`kings_bounty/` owns the native-game research: routine names, overworld mapping
and game asset extraction. No native Rust game crate existed at the split.

## Paths relative to mdx

| Path | Contents |
| --- | --- |
| `crates/` | Canonical mdcore and mdx source; optional real-cartridge gates |
| `docs/`, `notes/`, `external_agent_notes/` | Development logs, reviews and evidence |
| `testdata/` | Frozen reference images, replay inputs and their manifest |
| `tools/` | Observers, cartridge survey, smoke drivers and timing probes |
| `rom/original/` | Local test ROMs, including `kings_bounty.md` (ignored) |
| `rom/library/` | Downloaded test corpus (ignored) |
| `rom/archives/`, `rom/dos/` | Original archives and DOS reference files (ignored) |
| `cache/`, `.mdx/`, `.venv/`, `target/` | Local oracles, artifacts and builds (ignored) |
| `.legacy/reverse_kings/` | Old Git history, modified source, old builds and collided scripts (ignored) |

The archive preserves uncommitted source and the original Git history, which
already contained ROMs. It must remain local. No history was rewritten and no
commits were created. The move list is in `reorganisation-manifest.json`.
Historical evidence JSON, binary captures and hashes were left unchanged; paths
embedded in those records may still identify their original capture location.
Use the map above to locate them. The source template and its build output remain
under `templates/megadrive-bevy/`; no generic demo was relabelled as King's Bounty.

## Validation commands

```sh
cargo build -p mdx --release --locked
cargo test --workspace --release --locked
python3 -B tools/test_observers.py
cargo test -p mdx --release --locked --test kings_bounty_boot --test cadash_boot --test cartridge_fidelity -- --ignored
python3 tools/mcp_smoke.py --walking --out .mdx/reorganisation/mcp-smoke.json
```

The optional gates use this checkout's executable by default; `MDX_GOLDEN_BINARY`
can still select an explicit binary. The cartridge survey and research-probe
Cargo dependencies now resolve within this checkout.

## Global Codex registration

Run `tools/install-codex-mcp.sh` in a session that can write `~/.codex/config.toml`.
It registers this checkout's release binary as the global `mdx` stdio MCP server,
with this checkout as its default analysis/artifact root. Use absolute paths to
analyse or generate files in another workspace. Restart Codex to load new tools.

Registration was attempted during reorganisation, but the session mounts
`~/.codex` read-only. The global configuration has therefore not been changed.
The installer is ready; it does not change the sandbox or other MCP entries.

Official setup reference: https://learn.chatgpt.com/docs/extend/mcp?surface=cli

## Results from this reorganisation

Release build passed. The default Rust suite passed 178 tests with 22 explicitly
ignored tests; all four Python observer checks passed. A real MCP session listed
18 tools and exercised four cartridges plus walking/graphics/watch operations.
All 2,780 ROM images under `rom/` are ignored by Git, and all 27 pinned
ROM/replay/reference identities in the optional fidelity manifest still match.
See `reorganisation-validation.json` for the final cartridge-gate results and
`.mdx/reorganisation/` for local verification reports.

All 13 optional cartridge tests passed: the 12 original image gates and the
17-checkpoint four-cartridge fidelity test (27 distinct image checkpoints).
