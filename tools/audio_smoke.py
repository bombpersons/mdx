#!/usr/bin/env python3
"""Check real-cartridge audio continuation through CLI and actual MCP stdio.

No expected PCM is manufactured: concatenated split captures must exactly equal
an uninterrupted capture, including their complete final machine state. Original
ROMs and optional external reference WAVs are only read.
"""
import argparse
import array
import hashlib
import json
import math
from pathlib import Path
import subprocess
import wave
from mcp_smoke import Client


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def pcm(report):
    artifact = report['data']['wav_artifact']
    assert digest(artifact['path']) == artifact['sha256']
    with wave.open(artifact['path'], 'rb') as w:
        assert (w.getframerate(), w.getnchannels(), w.getsampwidth()) == (44100, 2, 2)
        assert w.getnframes() == report['data']['sample_frames']
        return w.readframes(w.getnframes())


def stats(path):
    with wave.open(str(path), 'rb') as w:
        data = array.array('h', w.readframes(w.getnframes()))
        if __import__('sys').byteorder != 'little':
            data.byteswap()
        result = {'sha256': digest(path), 'sample_frames': len(data)//2,
                  'peak': max(map(abs, data), default=0),
                  'rms': math.sqrt(sum(x*x for x in data)/max(1, len(data)))}
    try:
        import numpy as np
        mono = np.array(data, dtype=float).reshape(-1, 2).mean(axis=1)
        mono -= mono.mean()
        spectrum = np.abs(np.fft.rfft(mono*np.hanning(len(mono))))
        indexes = np.argsort(spectrum)[-8:][::-1]
        result['strongest_bins_hz'] = (indexes*44100/len(mono)).round(2).tolist()
    except ImportError:
        pass
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=Path('target/release/mdx'))
    p.add_argument('--rom', type=Path, required=True)
    p.add_argument('--start-frame', type=int, default=1900)
    p.add_argument('--frames', type=int, default=60)
    p.add_argument('--split', type=int, default=30)
    p.add_argument('--reference', type=Path)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    binary, rom, out = [path.resolve() for path in (args.binary, args.rom, args.out)]
    assert 0 < args.split < args.frames and args.frames <= 3600
    out.mkdir(parents=True, exist_ok=False)
    store = out/'.mdx/artifacts'
    common = {'project_root': str(out), 'rom': str(rom), 'instructions': 100000000,
              'save_state': True, 'out': 'waves'}

    def cli(operation, values, label):
        run = subprocess.run([str(binary), '--root', str(out), 'call', operation,
                              '--args', json.dumps(values)], capture_output=True, check=True)
        report = json.loads(run.stdout)
        assert report['ok'], report
        (out/(label+'.json')).write_text(json.dumps(report, indent=2)+'\n')
        return report

    whole = cli('audio', {**common, 'start_frame': args.start_frame, 'frames': args.frames}, 'uninterrupted')
    first = cli('audio', {**common, 'start_frame': args.start_frame, 'frames': args.split}, 'first')
    client = Client(binary, out, store)
    try:
        second = client.call('audio', {**common, 'from_state': first['data']['state_artifact']['id'],
                                      'frames': args.frames-args.split})
        assert second['ok'], second
        (out/'second.json').write_text(json.dumps(second, indent=2)+'\n')
        assert pcm(first)+pcm(second) == pcm(whole)
        assert first['data']['next_sample_index'] == second['data']['first_sample_index']
        assert second['data']['state_artifact']['id'] == whole['data']['state_artifact']['id']
        # Checkpoints created through ordinary boot must retain the same synthesis
        # phase, even though boot retains no PCM at all.
        warmup = cli('boot', {k: v for k, v in {**common, 'frames': args.start_frame,
                      'trace': 0}.items() if k != 'out'}, 'boot-checkpoint')
        resumed = client.call('audio', {**common, 'from_state': warmup['data']['state_artifact']['id'],
                                       'frames': args.frames})
        assert resumed['ok'], resumed
        assert resumed['data']['wav_artifact']['id'] == whole['data']['wav_artifact']['id']
        assert resumed['data']['state_artifact']['id'] == whole['data']['state_artifact']['id']
        loaded = client.call('ram.dump', {k: v for k, v in {**common,
                'from_state': whole['data']['state_artifact']['id'], 'instructions': 0}.items() if k != 'out'})
        assert loaded['data']['state_artifact']['id'] == whole['data']['state_artifact']['id']
        info = {'schema': 'mdx.audio-smoke.v1', 'binary_sha256': digest(binary),
                'rom_sha256': digest(rom), 'start_frame': args.start_frame, 'frames': args.frames,
                'split': args.split, 'split_pcm_equal': True, 'split_final_state_equal': True,
                'boot_checkpoint_pcm_equal': True, 'zero_step_state_equal': True,
                'advertised_tools': len(client.tools), 'output': stats(whole['data']['wav_artifact']['path']),
                'state_id': whole['data']['state_artifact']['id'], 'wav_id': whole['data']['wav_artifact']['id'],
                'scope': 'finite mdcore continuation and nonzero audio; no hardware/analog PCM equivalence'}
        assert info['output']['peak'] > 0
        if args.reference:
            info['reference'] = stats(args.reference)
            assert info['output']['sample_frames'] == info['reference']['sample_frames']
        (out/'summary.json').write_text(json.dumps(info, indent=2)+'\n')
        print(json.dumps(info, indent=2))
    finally:
        client.close()


if __name__ == '__main__':
    main()
