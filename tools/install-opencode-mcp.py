#!/usr/bin/env python3
"""Point OpenCode's global mdx MCP entry at this checkout, preserving other settings."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if args.config:
        config = args.config.expanduser().resolve()
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))) / "opencode"
        config = next((p for p in (base / "opencode.jsonc", base / "opencode.json") if p.is_file()), base / "opencode.json")
    binary = root / "target/release/mdx"
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit(f"Build mdx first: cargo build --manifest-path {root / 'Cargo.toml'} --release --locked -p mdx")
    original = config.read_text() if config.exists() else "{}"
    try:
        data = json.loads(original)
    except json.JSONDecodeError as error:
        raise SystemExit(f"Refusing to rewrite a config with comments/nonstandard JSON: {config}: {error}")
    mcp = data.setdefault("mcp", {})
    entry = mcp.setdefault("mdx", {"type": "local", "enabled": True})
    if entry.get("type", "local") != "local":
        raise SystemExit("Existing mdx entry is remote; refusing to replace it automatically")
    command = [str(binary), "mcp"]
    entry["command"] = command
    print(json.dumps({"config": str(config), "mdx": {"type": entry.get("type", "local"), "command": command, "enabled": entry.get("enabled", True)}, "dry_run": args.dry_run}, indent=2))
    if args.dry_run:
        return
    if config.exists() and json.loads(original) == data:
        print("OpenCode already uses this mdx checkout.")
        return
    # An atomic replacement keeps unrelated MCP entries and settings intact.
    mode = stat.S_IMODE(config.stat().st_mode) if config.exists() else 0o600
    config.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", dir=config.parent, prefix=".mdx-update-", delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(data, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, mode)
        if config.exists():
            if config.read_text() != original:
                raise RuntimeError("OpenCode config changed while updating; retry")
            stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
            backup = config.with_name(config.name + f".mdx-backup-{stamp}")
            shutil.copy2(config, backup)
            print(f"Backup: {backup}")
        os.replace(temporary, config)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print("Updated OpenCode. Restart it to reload mdx.")


if __name__ == "__main__":
    main()
