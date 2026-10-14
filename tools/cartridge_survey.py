#!/usr/bin/env python3
"""Finite, same-frame mdx/GPGX image and normalized-memory comparisons."""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from PIL import Image
from oracle_vdp_snapshot import normalize

ROOT = Path(__file__).resolve().parents[1]
ROMS = {
    "hokuto": "rom/original/hokuto/Hokuto no Ken - Shin Seikimatsu Kyuuseishu Densetsu (Japan).md",
    "cadash": "rom/original/Cadash (USA, Korea).md",
    "ewj": "rom/original/ewj/Earthworm Jim (USA).md",
    "kb": "rom/original/kings_bounty.md",
}
CASES = [
    ("hokuto", 60, None), ("hokuto", 300, None),
    ("hokuto", 1200, None), ("hokuto", 2800, "cross_rom_start_replay.json"),
    ("cadash", 600, None), ("cadash", 5500, "cadash_exit_replay.json"),
    ("ewj", 1200, None), ("ewj", 2800, "cross_rom_start_replay.json"),
    ("ewj", 6000, "ewj_play_replay.json"),
    ("kb", 1800, None), ("kb", 4500, "kb_walk_replay.json"),
]
EXTENDED_CASES = [
    ("hokuto", 4000, "hokuto_extended_replay.json"),
    ("hokuto", 6000, "hokuto_extended_replay.json"),
    ("cadash", 7000, "cadash_exit_replay.json"),
    ("ewj", 7000, "ewj_extended_replay.json"),
    ("ewj", 8000, "ewj_extended_replay.json"),
    ("kb", 6000, "kb_walk_replay.json"),
]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def run_case(case, binary, output):
    game, frame, replay = case
    directory = output / f"{game}-{frame}"
    directory.mkdir(parents=True)
    rom = ROOT / ROMS[game]
    input_args = ["--input", str(ROOT / "testdata" / replay)] if replay else []
    args = [str(binary), "boot", "--rom", str(rom), "--frames", str(frame),
            "--instructions", "100000000", "--trace", "0", "--out", str(directory / "mdx")]
    result = subprocess.run(args + input_args, cwd=ROOT, capture_output=True, text=True)
    (directory / "mdx.json").write_text(result.stdout)
    if result.returncode:
        raise RuntimeError(f"mdx failed for {game}/{frame}: {result.stdout} {result.stderr}")
    report = json.loads(result.stdout)["data"]
    oracle = directory / "oracle.png"
    state = directory / "oracle.state"
    result = subprocess.run([sys.executable, str(ROOT / "tools/oracle_gpgx.py"), str(rom),
                             "--frames", str(frame), "--out", str(oracle), "--state", str(state)]
                            + input_args, cwd=ROOT, capture_output=True, text=True, check=True)
    (directory / "oracle.log").write_text(result.stdout + result.stderr)
    snapshot = normalize(state, directory / "oracle-snapshot")
    image = Path(report["display"]["screenshot_path"])
    left, right = Image.open(image).convert("RGB"), Image.open(oracle).convert("RGB")
    if left.size != right.size:
        raise ValueError(f"image geometry differs: {left.size} vs {right.size}")
    a_pixels = list(zip(*[iter(left.tobytes())] * 3))
    b_pixels = list(zip(*[iter(right.tobytes())] * 3))
    differences = [(i % left.width, i // left.width, a, b)
                   for i, (a, b) in enumerate(zip(a_pixels, b_pixels)) if a != b]
    delta = Image.new("RGB", left.size)
    for x, y, a, b in differences:
        delta.putpixel((x, y), tuple(abs(c - d) for c, d in zip(a, b)))
    delta.save(directory / "difference.png")
    memories = {}
    for name in ("wram", "vram", "cram", "vsram"):
        a = Path(report["memory_artifacts"][name]["path"]).read_bytes()
        b = Path(snapshot["memory_artifacts"][name]["path"]).read_bytes()
        if len(a) != len(b):
            raise ValueError(f"{name} lengths differ")
        changed = [i for i, (x, y) in enumerate(zip(a, b)) if x != y]
        memories[name] = {"changed_bytes": len(changed), "first_offsets": changed[:16]}
    record = {"game": game, "frame": frame, "input": replay,
              "rom_sha256": digest(rom), "input_sha256": digest(ROOT / "testdata" / replay) if replay else None,
              "changed_pixels": len(differences), "pixel_bounds":
              [min(x[0] for x in differences), min(x[1] for x in differences),
               max(x[0] for x in differences), max(x[1] for x in differences)] if differences else None,
              "pixel_samples": differences[:8], "memory": memories,
              "registers_equal": report["vdp"]["registers"] == json.loads(Path(snapshot["snapshot"]).read_text())["vdp"]["registers"],
              "exceptions": report["exceptions"], "directory": str(directory),
              "image": str(image), "oracle": str(oracle),
              "image_sha256": digest(image), "oracle_sha256": digest(oracle),
              "oracle_state_sha256": digest(state),
              "reached_frame": report["vdp"]["frame"],
              "instructions": report["instructions"]}
    (directory / "comparison.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record), flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/mdx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--games", nargs="+", choices=ROMS)
    parser.add_argument("--jobs", type=int, default=3)
    parser.add_argument("--extended", action="store_true")
    parser.add_argument("--all", action="store_true", help="both original survey and extended checkpoints")
    parser.add_argument("--verify", action="store_true", help="fail on image, video-buffer, register or exception discrepancies")
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    selected = CASES + EXTENDED_CASES if args.all else (EXTENDED_CASES if args.extended else CASES)
    cases = [case for case in selected
             if not args.games or case[0] in args.games]
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        records = list(pool.map(lambda case: run_case(case, args.binary.resolve(), output), cases))
    summary = {"binary_sha256": digest(args.binary), "cases": records,
               "limitations": ["finite same-frame replays, not whole-game/audio/cycle equivalence",
                               "final memory comparison does not reconstruct raster history"]}
    (output / "survey.json").write_text(json.dumps(summary, indent=2) + "\n")
    if args.verify and any(record["changed_pixels"] or record["exceptions"]
                           or not record["registers_equal"] or record["reached_frame"] != record["frame"]
                           or any(record["memory"][name]["changed_bytes"] for name in ("vram", "cram", "vsram"))
                           for record in records):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
