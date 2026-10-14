#!/usr/bin/env bash
# Register this checkout as a global Codex MCP tool.
set -euo pipefail
mdx_checkout="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
mdx_binary="$mdx_checkout/target/release/mdx"
if [[ ! -x "$mdx_binary" ]]; then
    printf 'Build mdx first: cargo build --manifest-path %q --release --locked -p mdx\n' "$mdx_checkout/Cargo.toml" >&2
    exit 1
fi
codex mcp add mdx -- "$mdx_binary" mcp
codex mcp get mdx
