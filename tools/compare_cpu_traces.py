#!/usr/bin/env python3
"""Find the first exact state difference in instruction-aligned CPU traces.

The explicit start index excludes only a declared prefix (e.g. reset CCR is
unspecified); no fuzzy resynchronization, register normalization, or sorting.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import struct

NAMES = ["pc", "sr"] + [f"d{i}" for i in range(8)] + [f"a{i}" for i in range(8)]
RECORD = struct.Struct("<18I")


def identity(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    size = path.stat().st_size
    if size % RECORD.size:
        raise ValueError(f"incomplete trace record in {path}")
    return {"path": str(path), "sha256": digest.hexdigest(), "records": size // RECORD.size}


def state(record):
    return {name: f"0x{value:08X}" for name, value in zip(NAMES, record)}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("left", type=pathlib.Path)
    ap.add_argument("right", type=pathlib.Path)
    ap.add_argument("--start-index", type=int, default=0)
    args = ap.parse_args()
    if args.start_index < 0:
        raise SystemExit("start index must be nonnegative")
    result = {"left": identity(args.left), "right": identity(args.right),
              "excluded_prefix_records": args.start_index, "first_difference": None,
              "limitations": ["CPU state only; cycles and bus events are not compared",
                              "a finite equal prefix does not prove whole-program equivalence"]}
    context = collections.deque(maxlen=4)
    with args.left.open("rb") as left, args.right.open("rb") as right:
        for index in range(min(result["left"]["records"], result["right"]["records"])):
            a, b = RECORD.unpack(left.read(RECORD.size)), RECORD.unpack(right.read(RECORD.size))
            if index >= args.start_index and a != b:
                result["first_difference"] = {
                    "index": index, "left": state(a), "right": state(b),
                    "fields": [name for name, x, y in zip(NAMES, a, b) if x != y],
                    "preceding": list(context),
                }
                break
            context.append({"index": index, "left": state(a), "right": state(b)})
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
