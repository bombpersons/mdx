#!/usr/bin/env python3
"""Capture independent reference WAV audio with all earlier frames emulated.

Requires the caller's separately obtained GPGX core; no reference execution is
modified. This is validation audio, not mdcore's pending synthesis feature.
"""
import argparse
import hashlib
import json
from pathlib import Path
import oracle_gpgx as oracle
from pcm_capture import WaveCapture


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--rom', type=Path, required=True)
    ap.add_argument('--input', type=Path)
    ap.add_argument('--start-frame', type=int, default=0)
    ap.add_argument('--frames', type=int, required=True, help='number of frames retained')
    ap.add_argument('--out', type=Path, required=True, help='new output directory')
    args = ap.parse_args()
    if args.start_frame < 0 or args.frames < 1:
        ap.error('start-frame must be nonnegative and frames positive')
    events = json.loads(args.input.read_text())['events'] if args.input else []
    args.out.mkdir(parents=True, exist_ok=False)
    wave = args.out / 'reference.wav'
    capture = WaveCapture(wave, args.start_frame, args.start_frame + args.frames)
    try:
        oracle.run(args.rom.resolve(), args.start_frame + args.frames, events, capture=capture)
    finally:
        capture.close()
    if capture.frames == 0:
        raise RuntimeError('reference core produced no retained audio')
    sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    report = {'schema':'mdx.reference-audio.v1','source':'unchanged GPGX libretro callback PCM',
              'rom_sha256':sha(args.rom),'core_sha256':sha(oracle._find_core()),
              'input_sha256':sha(args.input) if args.input else None,
              'start_frame':args.start_frame,'frames':args.frames,
              'wav_path':str(wave.resolve()),'wav_sha256':sha(wave),
              **capture.report(),
              'limitations':['reference output, not mdcore synthesis or hardware equivalence',
                             'zero-based frame interval; preceding audio warmup retained internally']}
    (args.out / 'report.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
