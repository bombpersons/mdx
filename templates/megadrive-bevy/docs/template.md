# Template edits

`md-bevy/` comes from the mdx template (`.mdx-template.toml` records the
version). Edits to it are allowed under the rules in `md-bevy/AGENTS.md`;
this file lists them so that `mdx template update` stays easy to merge.

```sh
mdx template update --dry-run
mdx template update
```

Each edited file that the template also changed comes back as
`<file>.mdx-new`. Take the new version, reapply the change listed below (or
drop the row if the template now does it), then delete the `.mdx-new`.

## Local edits

| File | Change | Why | Raised upstream |
|---|---|---|---|
| _example: `md-bevy/src/look/plane.rs`_ | _adds a per-row scroll hook_ | _the original's parallax needs it (`docs/evidence.md`)_ | _no_ |

## Workarounds

Template limits handled from `engine/` or `game/` without editing it, and
whether they should be raised upstream.

| Limit | Workaround | Raised upstream |
|---|---|---|
