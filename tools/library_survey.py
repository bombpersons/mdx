#!/usr/bin/env python3
"""Bounded CLI survey of the ignored ROM library; findings are triage, not fidelity.

All original images and manifests are read-only. Each output directory records
binary/ROM identities, requested scope, complete responses and compact JSONL rows.
No reference emulator is involved in this first-pass smoke survey.
"""
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
from pathlib import Path
import subprocess
import shutil
import time

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as file:
        for chunk in iter(lambda: file.read(1024*1024), b''):
            h.update(chunk)
    return h.hexdigest()


def inventory(manifest):
    source = json.loads(manifest.read_text())
    for archive in source['archives']:
        for rom in archive['roms']:
            # The historical manifest predates the workspace move. Resolve its
            # archive/member structure in place without rewriting its identity.
            path = manifest.parent/'roms'/Path(archive['archive']).stem/rom['member']
            if not path.resolve().is_relative_to((manifest.parent/'roms').resolve()):
                raise ValueError('ROM member escapes the library')
            yield {'name': rom['member'], 'path': str(path.resolve()),
                   'sha256': rom['sha256'], 'bytes': rom['bytes']}


def run(case, args, out):
    begin = time.monotonic()
    path = Path(case['path'])
    row = {**case, 'requested_frames': args.frames, 'instruction_budget': args.instructions}
    if not path.is_file() or digest(path) != case['sha256']:
        return {**row, 'classification': 'input_identity_mismatch'}
    with path.open('rb') as source:
        header = source.read(512)
    row['region_header'] = header[0x1f0:0x200].decode('ascii', errors='replace').strip('\x00 ')
    row['console_header'] = header[0x100:0x110].decode('ascii', errors='replace').strip('\x00 ')
    row['power_of_two_bytes'] = bool(case['bytes'] and not case['bytes'] & (case['bytes']-1))
    values = {'project_root': str(out), 'rom': str(path), 'frames': args.frames,
              'instructions': args.instructions, 'timeout_ms': args.timeout_ms,
              'trace': 0}
    try:
        result = subprocess.run([str(args.binary), 'call', 'boot', '--args', json.dumps(values)],
                                capture_output=True, text=True, timeout=args.process_timeout)
    except subprocess.TimeoutExpired:
        return {**row, 'classification': 'process_timeout', 'wall_seconds': time.monotonic()-begin}
    report_path = out/'reports'/(case['sha256']+'.json')
    report_path.write_text(result.stdout)
    if result.stderr:
        report_path.with_suffix('.stderr').write_text(result.stderr)
    row.update(returncode=result.returncode, wall_seconds=round(time.monotonic()-begin, 4),
               report=str(report_path.relative_to(out)))
    try:
        report = json.loads(result.stdout)
        if 'result_artifact' in (report.get('data') or {}):
            artifact = report['data']['result_artifact']
            p = Path(artifact['path'])
            if not p.resolve().is_relative_to(out):
                raise ValueError('result artifact outside selected workspace')
            if digest(p) != artifact['sha256']:
                raise ValueError('result artifact digest mismatch')
            report = json.loads(p.read_text())
        if report.get('error'):
            return {**row, 'classification': 'operation_error', 'error': report['error']}
        if report['subject']['sha256'] != case['sha256']:
            return {**row, 'classification': 'input_identity_mismatch'}
        data = report['data']
        row.update(reached_frames=data['reached_frames'],
                   frame_target_reached=data['frame_target_reached'],
                   termination=data['termination'], steps=data['instructions'],
                   exceptions=data['exceptions'],
                   last_instruction_next_pc=(data.get('last_addresses') or [{}])[-1].get('next_pc'),
                   display_enabled=data['vdp']['display_enabled'],
                   distinct_colours=data['display']['distinct_colours'],
                   unmapped_accesses=data['unmapped_accesses'],
                   memory_sha256={k: v['sha256'] for k, v in data['memory_artifacts'].items()},
                   screenshot_sha256=data['display']['screenshot_artifact']['sha256'],
                   # Encoding-independent; prefer this when comparing runs.
                   screenshot_pixels_sha256=data['display'].get('screenshot_pixels_sha256'))
        row['classification'] = ('scope_incomplete' if not data['frame_target_reached']
                                 else 'software_traps_observed' if data['exceptions'] and all(e['kind'] in {f'vector_{n}' for n in range(32,48)} for e in data['exceptions'])
                                 else 'exceptions_observed' if data['exceptions']
                                 else 'frame_target_reached')
    except (ValueError, KeyError, TypeError) as error:
        row.update(classification='invalid_response', error=str(error))
    return row


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest', type=Path, default=ROOT/'rom/library/nointro-md/manifest.json')
    p.add_argument('--binary', type=Path, default=ROOT/'target/release/mdx')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--frames', type=int, default=8)
    p.add_argument('--instructions', type=int, default=500000)
    p.add_argument('--timeout-ms', type=int, default=3000)
    p.add_argument('--process-timeout', type=float, default=10)
    p.add_argument('--jobs', type=int, default=4)
    p.add_argument('--select', type=Path, help='JSON array of exact ROM names or SHA-256 digests')
    p.add_argument('--limit', type=int)
    args = p.parse_args()
    if not (1 <= args.frames <= 100000 and 1 <= args.instructions <= 100000000 and 1 <= args.jobs <= 16):
        p.error('frames/steps/jobs outside operation bounds')
    args.binary = args.binary.resolve()
    out = args.out.resolve(); out.mkdir(parents=True, exist_ok=False)
    (out/'reports').mkdir()
    cases = list(inventory(args.manifest.resolve()))
    if args.select:
        selected = set(json.loads(args.select.read_text()))
        cases = [c for c in cases if c['name'] in selected or c['sha256'] in selected]
        found = {value for c in cases for value in (c['name'], c['sha256'])}
        if selected-found:
            raise ValueError(f'selections missing from manifest: {sorted(selected-found)}')
    if args.limit is not None:
        cases = cases[:args.limit]
    counts = Counter(); records = []; begin = time.monotonic()
    identity = {'schema': 'mdx.library-survey.v1', 'binary_sha256': digest(args.binary),
                'manifest_sha256': digest(args.manifest), 'script_sha256': digest(Path(__file__)), 'cases': len(cases),
                'frames': args.frames, 'instructions': args.instructions,
                'timeout_ms': args.timeout_ms, 'jobs': args.jobs,
                'model': 'mdcore NTSC/USA default console; no forced branches or controller input',
                'limitations': ['smoke/exception triage only; no reference or hardware equivalence',
                                'blank/disabled early frames and observed exceptions are not automatically defects',
                                'region locks, unsupported mappers/add-on hardware and bad/prototype dumps need separate investigation']}
    shutil.copyfile(Path(__file__), out/'executed-survey.py')
    (out/'identity.json').write_text(json.dumps(identity, indent=2)+'\n')
    print(json.dumps(identity), flush=True)
    with (out/'results.jsonl').open('w') as log, ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(run, case, args, out) for case in cases]
        for future in as_completed(futures):
            row = future.result(); counts[row['classification']] += 1; records.append(row)
            log.write(json.dumps(row)+'\n'); log.flush()
            if len(records) % 100 == 0 or len(records) == len(cases):
                print(json.dumps({'completed': len(records), 'total': len(cases),
                                  'counts': counts, 'wall_seconds': round(time.monotonic()-begin, 1)}), flush=True)
    (out/'summary.json').write_text(json.dumps({**identity, 'counts': counts,
        'wall_seconds': round(time.monotonic()-begin, 2),
        'findings': [r for r in records if r['classification'] != 'frame_target_reached']}, indent=2)+'\n')


if __name__ == '__main__':
    main()
