#!/usr/bin/env python3
"""Find the first dispatch PC/clock divergence without retaining giant traces.

Uses an unchanged-execution, observation-instrumented oracle and mdcore's
timing_trace example. Stop both processes at the first difference; the partial
oracle screenshot is not a reference capture. Each pipe is drained separately
so independently buffered register/timing writes cannot deadlock one another.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import struct
import subprocess
import sys
import threading
from collections import deque

ROOT = Path(__file__).resolve().parents[2]


def records(stream, layout):
    chunks = queue.Queue(maxsize=8)
    size = struct.calcsize(layout)

    def drain():
        try:
            while chunk := stream.read(size*2048):
                if len(chunk) % size:
                    raise RuntimeError("incomplete trace record")
                chunks.put(chunk)
        except Exception as error:
            chunks.put(error)
        finally:
            chunks.put(None)

    threading.Thread(target=drain, daemon=True).start()
    while (chunk := chunks.get(timeout=120)) is not None:
        if isinstance(chunk, Exception):
            raise chunk
        yield from struct.iter_unpack(layout, chunk)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rom", type=Path, required=True)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=ROOT/"target/release/examples/timing_trace")
    parser.add_argument("--oracle-core", type=Path, required=True)
    parser.add_argument("--skip", type=int, default=0)
    parser.add_argument("--dispatches", type=int, required=True)
    parser.add_argument("--frames", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.skip < 0 or args.dispatches < 1 or args.frames < 1:
        parser.error("counts must be positive (skip may be zero)")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    cpu_read, cpu_write = os.pipe()
    time_read, time_write = os.pipe()
    cpu_stream = os.fdopen(cpu_read, "rb")
    time_stream = os.fdopen(time_read, "rb")
    env = {k: v for k, v in os.environ.items() if not k.startswith("MDX_ORACLE_TRACE")}
    env.update(MDX_ORACLE_CORE=str(args.oracle_core.resolve()),
               MDX_ORACLE_TRACE=f"/dev/fd/{cpu_write}",
               MDX_ORACLE_TRACE_TIMING=f"/dev/fd/{time_write}",
               MDX_ORACLE_TRACE_START_COUNT=str(args.skip),
               MDX_ORACLE_TRACE_LIMIT=str(args.dispatches))
    processes = []
    try:
        with (out/"oracle.log").open("w") as oracle_log, (out/"mdx.log").open("w") as mdx_log:
            oracle = subprocess.Popen([sys.executable, str(ROOT/"tools/oracle_gpgx.py"), str(args.rom.resolve()),
                                       "--frames", str(args.frames), "--input", str(args.input.resolve()),
                                       "--out", str(out/"partial-oracle.png")], cwd=ROOT, env=env,
                                      pass_fds=(cpu_write, time_write), stdout=oracle_log, stderr=oracle_log)
            processes.append(oracle)
            os.close(cpu_write)
            os.close(time_write)
            mdx = subprocess.Popen([str(args.binary.resolve()), str(args.rom.resolve()), str(args.dispatches),
                                    "/dev/stdout", str(args.input.resolve()), str(args.skip)], cwd=ROOT,
                                   stdout=subprocess.PIPE, stderr=mdx_log)
            processes.append(mdx)
            # Start all drains before consuming the first record.
            # records() is a generator, so explicitly advance each in a thread
            # by using unbuffered independent block queues in the iterators.
            def start(stream, layout):
                q = queue.Queue(maxsize=4096)
                def feed():
                    try:
                        for row in records(stream, layout): q.put(row)
                    except Exception as error: q.put(error)
                    finally: q.put(None)
                threading.Thread(target=feed, daemon=True).start()
                return q
            actual, reference, timing = (start(mdx.stdout, "<Q18I"),
                                         start(cpu_stream, "<18I"), start(time_stream, "<3I"))
            context = deque(maxlen=12)
            equal = 0
            mismatch = None
            for index in range(args.dispatches):
                values = [q.get(timeout=120) for q in (actual, reference, timing)]
                if any(isinstance(row, Exception) for row in values):
                    raise next(row for row in values if isinstance(row, Exception))
                if any(row is None for row in values):
                    raise RuntimeError(f"trace ended before requested scope at dispatch {args.skip+index}")
                ours, expected, at = values
                clock = at[1]*896040+at[2]
                row = {"ordinal": at[0], "frame": at[1], "mdx_master": ours[0],
                       "oracle_master": clock, "mdx_pc": ours[1], "oracle_pc": expected[0]}
                if ours[0] != clock or ours[1] != expected[0]:
                    mismatch = {**row, "mdx_registers": list(ours[1:]), "oracle_registers": list(expected)}
                    break
                equal += 1
                context.append(row)
                if equal % 1000000 == 0: print(json.dumps({"equal": equal, "frame": at[1]}), flush=True)
            digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
            result = {"equal_dispatches": equal, "skipped_dispatches": args.skip,
                      "first_difference": mismatch, "preceding": list(context),
                      "binary_sha256": digest(args.binary), "oracle_core_sha256": digest(args.oracle_core),
                      "rom_sha256": digest(args.rom), "input_sha256": digest(args.input),
                      "limitations": ["compares pre-dispatch PC/master clocks, not complete state or hardware behavior",
                                       "processes stop at first difference; partial output is not a reference image"]}
            (out/"summary.json").write_text(json.dumps(result, indent=2)+"\n")
            print(json.dumps({k: v for k, v in result.items() if k != "preceding"}), flush=True)
    finally:
        for process in processes:
            if process.poll() is None: process.terminate()
        for process in processes:
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
    if mismatch is not None:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
