#!/usr/bin/env python3
"""Convert mdx instruction JSONL to the oracle's pc,sr,D0-D7,A0-A7 layout.

Adapted from the other agent's diagnostic. Preserves order and raw state;
publishes input/output digests. Uses a fresh output path to avoid stale traces.
"""
import argparse
import hashlib
import json
import pathlib
import struct

RECORD = struct.Struct("<18I")


def convert(source, output):
    raw = source.read_bytes()
    if len(raw) > 64 * 1024 * 1024:
        raise ValueError("trace input exceeds 64 MiB")
    encoded = bytearray()
    records = 0
    for line in raw.splitlines():
        if not line.strip():
            continue
        event = json.loads(line)
        if len(event["d"]) != 8 or len(event["a"]) != 8:
            raise ValueError("expected eight data and eight address registers")
        values = [event["pc"], event["sr"], *event["d"], *event["a"]]
        if any(type(value) is not int or not 0 <= value <= 0xffffffff for value in values):
            raise ValueError("trace register must be an unsigned 32-bit integer")
        encoded.extend(RECORD.pack(*values))
        records += 1
        if records > 100000:
            raise ValueError("trace input exceeds 100000 records")
    with output.open("xb") as target:
        target.write(encoded)
    return {"records": records, "source_sha256": hashlib.sha256(raw).hexdigest(),
            "output": str(output.resolve()), "output_sha256": hashlib.sha256(encoded).hexdigest(),
            "limitations": ["CPU state only; no timing/memory equivalence claim"]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    print(json.dumps(convert(args.artifact, args.output)))
