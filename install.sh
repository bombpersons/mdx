#!/bin/sh
# Install mdx from a GitHub release (Linux and macOS):
#
#   curl -fsSL https://raw.githubusercontent.com/bombpersons/mdx/main/install.sh | sh
#
# Settings (environment variables):
#   MDX_VERSION=v0.1.0      a specific release instead of the latest
#   MDX_INSTALL_DIR=DIR     where the binary goes (default ~/.local/bin)
#   MDX_NO_MCP=1            don't register mdx with Claude Code, Codex and OpenCode
#   MDX_DOWNLOAD_BASE=URL   fetch the archives from here instead (a mirror, or file:// for testing)
#
# Running it again updates mdx. Nothing else is downloaded: ROMs are yours
# to supply.
set -eu

repo=bombpersons/mdx

say() { printf '%s\n' "$@"; }
fail() { printf 'mdx install: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64 | Linux-amd64) target=x86_64-unknown-linux-gnu ;;
    Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
    Darwin-arm64) target=aarch64-apple-darwin ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    *) fail "no prebuilt binary for $(uname -s) $(uname -m); build from source with: cargo install --locked --git https://github.com/$repo mdx" ;;
esac
command -v curl > /dev/null || fail "curl is required"

if [ -n "${MDX_DOWNLOAD_BASE:-}" ]; then
    base=$MDX_DOWNLOAD_BASE
elif [ -n "${MDX_VERSION:-}" ]; then
    base=https://github.com/$repo/releases/download/$MDX_VERSION
else
    base=https://github.com/$repo/releases/latest/download
fi
asset=mdx-$target.tar.gz
dir=${MDX_INSTALL_DIR:-$HOME/.local/bin}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
say "Downloading $asset (${MDX_VERSION:-latest release})"
curl -fsSL -o "$tmp/$asset" "$base/$asset" || fail "download failed: $base/$asset"
curl -fsSL -o "$tmp/$asset.sha256" "$base/$asset.sha256" || fail "download failed: $base/$asset.sha256"
expected=$(cut -d ' ' -f 1 < "$tmp/$asset.sha256")
if command -v sha256sum > /dev/null; then
    actual=$(sha256sum "$tmp/$asset" | cut -d ' ' -f 1)
else
    actual=$(shasum -a 256 "$tmp/$asset" | cut -d ' ' -f 1)
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch for $asset"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$dir"
cp "$tmp/mdx" "$dir/mdx.new"
chmod 755 "$dir/mdx.new"
mv -f "$dir/mdx.new" "$dir/mdx"
say "Installed $("$dir/mdx" --version) to $dir/mdx"

if [ -z "${MDX_NO_MCP:-}" ]; then
    say "" "Registering mdx as an MCP server (set MDX_NO_MCP=1 to skip):"
    "$dir/mdx" setup mcp || say "Some clients could not be set up; see above."
fi

case ":$PATH:" in
    *":$dir:"*) mdx=mdx ;;
    *) mdx=$dir/mdx; say "" "$dir is not on your PATH; add it, or use the full path below." ;;
esac
say "" "Next:" \
    "  $mdx header --rom /path/to/game.md      # try it on a cartridge" \
    "  $mdx guide port                         # porting a game to native code" \
    "  $mdx setup mcp                          # register again after installing an agent client"
