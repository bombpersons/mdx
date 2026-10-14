#!/usr/bin/env python3
"""Fetch and smoke-test the pinned optional FM backend into ignored local cache.

Run where crates.io is reachable. This does not change the workspace manifest,
global Cargo configuration or the installed mdx binary. The compiled probe checks
the backend API before it is integrated into the engine.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    folder = ROOT / '.mdx/audio-backend'
    probe = folder / 'probe'
    (probe / 'src').mkdir(parents=True, exist_ok=True)
    (probe / 'Cargo.toml').write_text(
        '[package]\nname="mdx-audio-backend-probe"\nversion="0.0.0"\n'
        'edition="2021"\n[workspace]\n[dependencies]\nymfm-sys="=0.2.0"\n')
    (probe / 'src/main.rs').write_text('''use ymfm_sys::ffi::{create_chip,ChipType};
fn main() {
    let mut c=create_chip(ChipType::Ym2612,7_670_454);
    let register=|chip:&mut ymfm_sys::ChipPtr,address:u8,value:u8| {
        chip.pin_mut().write(0,address);chip.pin_mut().write(1,value);
    };
    for slot in 0..4 {
        register(&mut c,0x30+slot*4,1);
        register(&mut c,0x40+slot*4,0);
        register(&mut c,0x50+slot*4,0x1f);
        register(&mut c,0x80+slot*4,0x0f);
    }
    register(&mut c,0xb0,7);register(&mut c,0xb4,0xc0);
    register(&mut c,0xa4,0x22);register(&mut c,0xa0,0x69);
    register(&mut c,0x28,0xf0);
    let mut b=vec![0i32;2048];
    c.pin_mut().generate(&mut b);
    assert!(b.iter().any(|x|*x!=0),"FM test tone is silent");
    let state=c.pin_mut().save_state();
    c.pin_mut().generate(&mut b);
    let expected=b.clone();
    c.pin_mut().restore_state(&state);
    c.pin_mut().generate(&mut b);
    assert_eq!(b,expected);
    println!("rate={} channels={} state_bytes={} continuation_equal=true",
        c.sample_rate(),c.channels(),state.len());
}
''')
    # Seed immutable existing registry caches by copying, never by linking writable
    # files back into the user's global Cargo directory.
    upstream = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
    local = folder / 'cargo'
    for name in ['index', 'cache']:
        source = upstream / 'registry' / name
        target = local / 'registry' / name
        if source.exists() and source.resolve() != target.resolve():
            shutil.copytree(source, target, dirs_exist_ok=True)
    env = {**os.environ, 'CARGO_HOME': str(local)}
    subprocess.run(['cargo','fetch','--manifest-path',str(probe/'Cargo.toml')],
                   cwd=ROOT, env=env, check=True)
    subprocess.run(['cargo','run','--offline','--release','--locked',
                    '--manifest-path',str(probe/'Cargo.toml')],
                   cwd=ROOT, env=env, check=True)
    print(json.dumps({'backend':'ymfm-sys','version':'0.2.0','cargo_home':str(local),
                      'manifest':str(probe/'Cargo.toml'),'download_and_probe_passed':True}))


if __name__ == '__main__':
    main()
