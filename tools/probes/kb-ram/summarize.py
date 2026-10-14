#!/usr/bin/env python3
"""Verify saved focused KB evidence; does not execute either emulator."""
import collections
import hashlib
import json
import pathlib
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / '.mdx/residual-review/kb'
FROZEN = ROOT / '.mdx/fidelity-review/release-verified'

def records(name):
    return [json.loads(line) for line in (OUT / name).read_text().splitlines()]

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    oracle = records('oracle-residual.jsonl')
    canonical = records('canonical-final.jsonl')
    cf = records('hv-ticks6000.jsonl')
    summary = {'gates': [], 'limitations': [
        'HV/tick substitution is a causal consumed-input experiment, not an emulator patch or cycle-equivalence proof.',
        'Affected-read examples retain the first four accesses per PC; dedicated saved-stack, HV, tick and sound handshake logs are not capped.',
        'Negative saved-stack consumption is bounded to this unchanged replay through frame 6000.',
        'Z80 condition and absolute-LD corrections do not close the three sound-buffer bytes; DAC handler remains independently divergent.',
    ]}
    for frame in (4500, 6000):
        frozen = FROZEN / f'kb-{frame}'
        prefix = f'hv-ticks{frame}'
        a = (OUT / f'canonical-final-{frame}.wram').read_bytes()
        b = (frozen / 'oracle-snapshot/wram.bin').read_bytes()
        c = (OUT / f'{prefix}.wram').read_bytes()
        baseline = next((frozen / 'mdx').glob('wram-*.bin')).read_bytes()
        assert a == baseline
        differences = [i for i in range(65536) if c[i] != b[i]]
        video = {m: (OUT / f'{prefix}.{m}').read_bytes() == (frozen / f'oracle-snapshot/{m}.bin').read_bytes() for m in ('vram', 'cram', 'vsram')}
        video['registers'] = json.loads((OUT / f'{prefix}.registers.json').read_text()) == json.loads((frozen / 'oracle-snapshot/snapshot.json').read_text())['vdp']['registers']
        rgb = (OUT / f'{prefix}.rgb').read_bytes()
        expected = Image.open(frozen / 'oracle.png').convert('RGB').tobytes()
        pixels = sum(rgb[i:i+3] != expected[i:i+3] for i in range(0, len(rgb), 3))
        gate = {'frame': frame, 'canonical_reproduces_frozen_wram': True,
            'baseline_changed_bytes': sum(x != y for x, y in zip(a,b)),
            'counterfactual_changed_bytes': len(differences),
            'counterfactual_remaining': [{'offset':hex(i),'actual':c[i],'oracle':b[i]} for i in differences],
            'generated_state_differences_at_or_above_9c32': sum(i >= 0x9c32 for i in differences),
            'video_equal': video, 'changed_pixels': pixels}
        assert gate['baseline_changed_bytes'] == 1518 and len(differences) == 32
        assert all(video.values()) and pixels == 0
        summary['gates'].append(gate)
    assert (OUT / 'oracle-residual.wram').read_bytes() == (FROZEN / 'kb-6000/oracle-snapshot/wram.bin').read_bytes()
    summary['private_oracle_reproduces_frozen_wram'] = True
    entries = [r for r in cf if r.get('pc') == 0x13b0c]
    originals = [r for r in oracle if r['kind'] == 'pre' and r['pc'] == 0x13b0c]
    returns = [r for r in cf if r.get('pc') == 0x13bb4]
    original_returns = [r for r in oracle if r['kind'] == 'pre' and r['pc'] == 0x13bb4]
    assert len(entries) == len(originals) == len(returns) == len(original_returns) == 2895
    assert all(a['seed'] == b['seed'] for a,b in zip(entries,originals))
    assert all(a['d'][0] == b['d'][0] for a,b in zip(returns,original_returns))
    summary['rng'] = {'calls':2895,'all_entry_seeds_equal':True,'all_returns_equal':True,
        'first_hv_reads':[r for r in oracle if r['kind']=='hv'][:4]}
    hv_only = records('hv-only4030.jsonl')
    summary['rng']['first_tick_discrepancy'] = {
        'canonical_hv_only':[r for r in hv_only if r.get('pc') in (0x13b44,0x13b78) and r.get('rng_call')==61],
        'oracle':[r for r in oracle if r['kind']=='tick' and r['rng_call']==61]}
    pointer_reads = [r for r in oracle if r['kind']=='read' and 0x8ac8 <= r['address']&65535 < 0x8acc]
    stack_reads = [r for r in oracle if r['kind']=='residual_read' and 0x8b00 <= r['address']&65535 < 0x8b68]
    summary['expired_saves'] = {'8ac8': {'reads':len(pointer_reads),'last_read':pointer_reads[-1],
        'reads_after_frame610':sum(r['frame']>610 for r in pointer_reads)},
        '8b00_8b68': {'reads':len(stack_reads),'last_read':stack_reads[-1],
        'reads_at_or_after4500':sum(r['frame']>=4500 for r in stack_reads)}}
    assert summary['expired_saves']['8ac8']['reads_after_frame610'] == 0
    assert summary['expired_saves']['8b00_8b68']['reads_at_or_after4500'] == 0
    summary['consumers'] = [r for r in oracle if r['kind']=='affected_read']
    sound = [r for r in oracle if r['kind']=='sound_handshake' and r['pc']==0x29de8]
    current_sound = [r for r in canonical if r.get('pc')==0x29de8]
    summary['sound'] = {'first_oracle_ready_block':next(r for r in sound if r['value']==23),
        'last_oracle_read':sound[-1], 'canonical_read_values':sorted({r['post_d'][1]&255 for r in current_sound}),
        'original_condition_regression':json.loads((OUT/'z80-conditions-original.json').read_text()),
        'fixed_condition_regression':json.loads((OUT/'z80-conditions-fixed.json').read_text())}
    private = records('z80-private-trace.jsonl'); independent = records('z80-oracle-trace.jsonl')
    first = next(i for i,(a,b) in enumerate(zip(private,independent)) if a['pc']!=b['pc'])
    summary['sound']['after_condition_and_ld_first_control_discrepancy'] = {
        'index':first,'private_pre':private[max(0,first-4):first+1],
        'oracle_pre':independent[max(0,first-4):first+1]}
    artifacts = [OUT/n for n in ('oracle-residual.jsonl','oracle-residual.wram','canonical-final.jsonl',
        'hv-ticks4500.jsonl','hv-ticks6000.jsonl','hv-only4030.jsonl','z80-private-trace.jsonl','z80-oracle-trace.jsonl')]
    summary['artifacts'] = [{'path':str(p.relative_to(ROOT)),'sha256':digest(p),'bytes':p.stat().st_size} for p in artifacts]
    summary['inputs'] = [{'path':str(p.relative_to(ROOT)),'sha256':digest(p)} for p in (ROOT/'rom/original/kings_bounty.md',ROOT/'testdata/kb_walk_replay.json')]
    (OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps({'gates':[(g['frame'],g['baseline_changed_bytes'],g['counterfactual_changed_bytes'],g['changed_pixels']) for g in summary['gates']],
        'rng_calls_equal':2895,'expired_save_reads_after_gates':0,'sound_condition_cases':129,
        'sound_condition_failures_original':summary['sound']['original_condition_regression']['failed'],
        'sound_condition_failures_fixed':summary['sound']['fixed_condition_regression']['failed']}))

if __name__ == '__main__':
    main()
