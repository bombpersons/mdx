#!/usr/bin/env python3
"""Authored flat-bus cases against unchanged external GPGX Z80 instruction execution.

PC is normalized to the resume address during HALT; final written bytes are
compared by address, independently of the emulator's word-store callback order.
This tests one instruction at a time with fixed data, not arbitrary programs,
Z80 bus subcycles, MEMPTR history, hardware timing or integrated sound scheduling.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parents[2]


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--source',type=Path,default=ROOT/'cache/oracle/Genesis-Plus-GX')
    ap.add_argument('--binary',type=Path,default=ROOT/'target/release/examples/z80_probe')
    ap.add_argument('--out',type=Path,required=True)
    ap.add_argument('--fixture',type=Path)
    args=ap.parse_args();out=args.out.resolve();out.mkdir(parents=True,exist_ok=False)
    headers=out/'include';headers.mkdir();(headers/'shared.h').write_text('#include <stdint.h>\n#include <stdio.h>\n#include <string.h>\n#include "z80.h"\n')
    source=args.source.resolve()/'core/z80';harness=ROOT/'tools/oracle_z80_probe.c';probe=out/'oracle-probe'
    build=subprocess.run(['cc','-O2','-DLSB_FIRST','-DINLINE=static inline','-I',str(headers),'-I',str(source),str(source/'z80.c'),str(harness),'-o',str(probe)],capture_output=True,text=True,check=True)
    (out/'build.log').write_text(build.stdout+build.stderr)
    cases=[(kind,op,f) for kind in range(7) for op in range(256) for f in [0,1,0x40,0x80,0x45,0xff]]
    request=''.join(f'{k} {o} {f}\n' for k,o,f in cases)
    oracle=subprocess.run([str(probe)],input=request,text=True,capture_output=True,check=True).stdout
    actual=subprocess.run([str(args.binary.resolve())],input=request,text=True,capture_output=True,check=True).stdout
    (out/'oracle.jsonl').write_text(oracle);(out/'actual.jsonl').write_text(actual)
    expected=[json.loads(x) for x in oracle.splitlines()];observed=[json.loads(x) for x in actual.splitlines()]
    assert len(expected)==len(observed)==len(cases)
    def memory(row):return sorted(dict(row['writes']).items())
    bad=[{'expected':x,'actual':y} for x,y in zip(expected,observed) if x['registers']!=y['registers'] or x['cycles']!=y['cycles'] or memory(x)!=memory(y)]
    def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
    meta={'schema':'mdx.z80-instruction-matrix.v1','source_z80_sha256':sha(source/'z80.c'),'source_header_sha256':sha(source/'z80.h'),'harness_sha256':sha(harness),'profile':'little endian; unchanged core; flat64KiB bus; fixed seeded registers and memory','pc':'resume address during HALT (reference stores HALT opcode PC internally)','memory':'final written bytes by address; callback ordering is not an equivalence claim','limitations':['single authored data pattern, six flag seeds per opcode','no MEMPTR history, IRQ, bus subcycles or integrated machine equivalence claim'],'total':len(cases),'equal':len(cases)-len(bad)}
    (out/'report.json').write_text(json.dumps({**meta,'binary_sha256':sha(args.binary),'differences':bad},indent=2)+'\n')
    if args.fixture:
        # Expected observations come exclusively from the unchanged external core.
        fixture={**meta,'cases':[{**{k:x[k] for k in ['kind','opcode','flags','cycles','registers']},'memory':memory(x)} for x in expected]}
        args.fixture.write_text(json.dumps(fixture,separators=(',',':'))+'\n')
    print(json.dumps(meta));assert not bad


if __name__=='__main__':main()
