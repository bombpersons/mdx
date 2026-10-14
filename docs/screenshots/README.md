# Verified King's Bounty milestones

These screenshots were rendered by `mdcore` from the unmodified Mega Drive
cartridge. Each image below matches the independent Genesis Plus GX capture
at all **71,680 RGB pixels**, with zero CPU exceptions:

| File | Completed frames | Replay |
| --- | ---: | --- |
| `frame-1200.png` | 1200 | No input: title screen |
| `frame-3600.png` | 3600 | `testdata/kb_character_replay.json` |
| `frame-3900.png` | 3900 | `testdata/kb_new_game_replay.json` |
| `frame-4300.png` | 4300 | `testdata/kb_world_replay.json` |

Reproduce the overworld capture:

```sh
cargo run -p mdx --release -- boot \
  --rom rom/original/kings_bounty.md \
  --frames 4300 --instructions 80000000 --trace 0 \
  --input testdata/kb_world_replay.json --out .mdx/world
```

The emulator is an analysis tool; these captures do not constitute the native
Rust game reconstruction. Moving credits and the rider's flag animation still
expose scheduling differences; those are recorded in `docs/investigation-log.md`.
