# Project-local analysis and concurrent agents

Every CLI/API/MCP operation accepts two optional common arguments:

- `project_root`: the calling agent's absolute, existing project directory.
- `artifact_store`: an optional absolute directory or a directory relative to
  that project's root. Use it to separate runs/agents when desired.

With `project_root`, relative ROM, replay, snapshot and output paths resolve
there, and the default artifact store is `<project_root>/.mdx/artifacts`.
The source checkout can live elsewhere. Use absolute ROM paths to reference the
ignored test cartridges in the mdx checkout without copying them into a project.
Template generation retains its explicit absolute-destination requirement.

For example, call MCP `boot` or CLI `mdx call boot --args` with:

```json
{
  "project_root": "/absolute/path/to/kings_bounty",
  "rom": "/absolute/path/to/mdx/rom/original/kings_bounty.md",
  "frames": 60,
  "instructions": 3000000,
  "trace": 0,
  "out": "analysis/first-boot"
}
```

The requested capture files go to the project's `analysis/first-boot/`; retained
snapshot, buffer and large-result objects go to its `.mdx/artifacts/objects/`.
`boot`, `render` and `graphics` accept `out`; the store also holds trace/watch
JSONL and artifact-ID objects. A single output folder is not forced on callers.

On subsequent snapshot/artifact calls, supply the same `project_root` and any
store override. IDs are immutable content identities, but their lookup is local
to the selected store. An absolute file path can still access a known artifact
from another project deliberately.

```json
{
  "project_root": "/absolute/path/to/kings_bounty",
  "snapshot": "sha256:THE_RETURNED_SNAPSHOT_ID",
  "memory": "wram",
  "address": 58700,
  "length": 16
}
```

Concurrent calls clone their context; they never change the process working
directory. One server can analyse two projects, each using `game.md`, without
mixing inputs, captures or artifact IDs. Surveys inherit their parent scope;
individual tasks can override it. Same-project agents can share a store safely:
publishing is atomic, existing objects are digest-verified and identical bytes
are deduplicated. For easier browsing, choose separate `out` directories or
`artifact_store: ".mdx/agents/worker-a"`. Isolation is optional, not necessary
to prevent content-addressed object overwrites.

Without `project_root`, calls retain the server/CLI launch context. CLI `--root`
and `--store` still set that context. A shared MCP server cannot reliably infer
which client project a request means, so agents should pass `project_root`.
The MCP instructions and advertised schemas explain these arguments.

Paths are JSON strings, including `project_root` and `artifact_store`: quote
the complete value and omit unused options. These two fields advertise the
simple `"type": "string"` schema, like `rom`, while remaining optional. This
avoids unnecessary nullable-type conversion in model/provider adapters. If a
client reports a JSON parse error before dispatch (for example an unquoted
`/home/...`), mdx has not received a tool call and cannot repair its arguments.
Reconnect the MCP server/restart OpenCode after rebuilding to refresh its
cached tool definitions. This compatibility adjustment does not establish
which layer produced an earlier malformed call.

## Codex and OpenCode registration

`mdx setup mcp` registers the running mdx executable with `mcp` (Claude Code
and Codex through their CLIs, OpenCode by editing its global config), without
pinning an unrelated project's root or store. It replaced the earlier
`tools/install-codex-mcp.sh` and `tools/install-opencode-mcp.py`. Launch-directory defaults work for dedicated clients; explicit
`project_root` remains reliable when the client reuses one server for projects.

The OpenCode update changes only mdx's entry, preserves other configuration
values and their order, and creates a timestamped backup. `--dry-run` prints
what would change. The existing global JSONC file is plain valid JSON; commented configs
are refused rather than being rewritten destructively.

The user has updated both global registrations and their binary paths were
verified. Workspace `opencode.json` overrides are also installed in the Mega
Drive workspace and mdx checkout (they point at the checkout's release build
and take precedence over the global entry there). For future moves:

```sh
mdx setup mcp --client opencode
```

Official configuration precedence: https://opencode.ai/docs/config/#per-project

Resumable machine-state artifacts use the same project-local store and immutable
IDs. See [save-states.md](save-states.md) for loading a shared checkpoint, relative
frame/replay semantics and transferring states between project stores.
