#!/usr/bin/env python3
"""Normalize a saved little-endian GPGX 1.7.6 MD state to mdx VDP buffers.

The layout is pinned to core/state.c::state_save and
core/vdp_ctrl.c::vdp_context_save in this project's oracle checkout.
This does not execute a target or infer scanline history from a final snapshot.
"""
import argparse
import hashlib
import json
import pathlib
import struct
from md_bytes import word_bytes


def normalize(source, output):
    state = source.read_bytes()
    if state[:16].rstrip(b"\0") != b"GENPLUS-GX 1.7.6":
        raise ValueError("unsupported state version; expected GPGX 1.7.6")
    # version + WRAM + ZRAM + zstate(u8) + zbank(u32) + IO + SAT
    offset = 16 + 65536 + 8192 + 1 + 4 + 16 + 1024
    if len(state) < offset + 65536 + 128 + 128 + 32:
        raise ValueError("incomplete MD save state")
    if output.exists():
        raise ValueError("output directory must be absent")
    wram = word_bytes(state[16:16 + 65536])
    vram = bytearray(state[offset:offset + 65536])
    vram[::2], vram[1::2] = vram[1::2], vram[::2]
    offset += 65536
    packed = struct.unpack_from("<64H", state, offset)
    if any(value > 0x1ff for value in packed):
        raise ValueError("palette does not match the selected MD state profile")
    cram = b"".join(struct.pack(">H", ((v & 0x1c0) << 3) | ((v & 0x38) << 2) | ((v & 7) << 1)) for v in packed)
    offset += 128
    vsram = bytearray(state[offset:offset + 80])
    vsram[::2], vsram[1::2] = vsram[1::2], vsram[::2]
    offset += 128
    registers = list(state[offset:offset + 32])
    output.mkdir(parents=True)
    artifacts = {}
    for name, content in [("wram", wram), ("vram", vram), ("cram", cram), ("vsram", vsram)]:
        path = output / f"{name}.bin"
        path.write_bytes(content)
        artifacts[name] = {"path": str(path.resolve()), "bytes": len(content),
                           "sha256": hashlib.sha256(content).hexdigest()}
    snapshot = {"vdp": {"registers": registers}, "memory_artifacts": artifacts,
                "source": {"path": str(source.resolve()), "sha256": hashlib.sha256(state).hexdigest(),
                            "profile": "GPGX 1.7.6 MD little-endian state layout; WRAM normalized to 68000 byte order"},
                "limitations": ["final snapshot only; scanline history and FIFO state are not reconstructed"]}
    path = output / "snapshot.json"
    path.write_text(json.dumps(snapshot, indent=2) + "\n")
    return {"snapshot": str(path.resolve()), "source": snapshot["source"], "memory_artifacts": artifacts}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("state", type=pathlib.Path)
    parser.add_argument("output_directory", type=pathlib.Path)
    args = parser.parse_args()
    print(json.dumps(normalize(args.state, args.output_directory), indent=2))
