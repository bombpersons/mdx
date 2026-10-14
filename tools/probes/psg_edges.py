#!/usr/bin/env python3
"""Compare pulse deltas and internal PSG phase with unchanged external execution.

No resampling/filtering claim. Config calls use unchanged100% gain/panning only
to advance through the public API. Original source is never altered or vendored.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parents[2]


def cases():
    rows=[]
    for channel in range(3):
        for period in [0,1,7,1023]:
            for volume in [0,5,10,15]:
                rows.append(dict(channel=channel,period=period,volume=volume))
    for mode in range(8):
        for volume in range(16):rows.append(dict(channel=3,mode=mode,volume=volume))
    for row in rows:
        channel=row['channel'];volume=row['volume'];ops=[(0,-2)]
        if channel<3:
            period=row['period'];ops.extend([(0,0x80+channel*32+(period&15)),(0,period>>4)])
        else:ops.extend([(0,0xcd),(0,0),(0,0xe0+row['mode'])])
        vol=0x90+channel*32;ops.append((0,vol+volume))
        ops.extend((clock,-1) for clock in [1,239,240,241,2400,100001])
        ops.extend([(100007,vol+(volume^7)),(100008,2),(100009,15),(100010,vol)])
        if channel==3:ops.extend([(130001,0xe0+((row['mode']+4)&7)),(130002,0xf3)])
        else:ops.extend([(130001,0x80+channel*32+3),(130002,0)])
        ops.extend((clock,-1) for clock in [130003,250003,600001])
        row['operations']=ops
    return rows


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary',type=Path,default=ROOT/'target/release/examples/psg_probe')
    ap.add_argument('--out',type=Path,required=True)
    ap.add_argument('--fixture',type=Path)
    args=ap.parse_args();args.out.mkdir(parents=True,exist_ok=False)
    include=args.out/'include';include.mkdir()
    (include/'shared.h').write_text('''#ifndef MDX_PSG_PROBE_SHARED
#define MDX_PSG_PROBE_SHARED
#include <stdint.h>
#include <string.h>
typedef uint8_t uint8;typedef uint16_t uint16;typedef uint32_t uint32;typedef int8_t int8;
#include "blip_buf.h"
#include "psg.h"
typedef struct {blip_t *blips[3];} mdx_sound;
typedef struct {int hq_psg;} mdx_config;
extern mdx_sound snd;extern mdx_config config;
#define save_param(p,n) do {memcpy(state+bufferptr,(p),(n));bufferptr+=(n);} while(0)
#define load_param(p,n) do {memcpy((p),state+bufferptr,(n));bufferptr+=(n);} while(0)
#endif
''')
    source=ROOT/'cache/oracle/Genesis-Plus-GX/core/sound';harness=ROOT/'tools/oracle_psg_probe.c'
    executable=args.out/'oracle-probe'
    build=subprocess.run(['cc','-O2','-I',str(include.resolve()),'-I',str(source),str(source/'psg.c'),str(harness),'-o',str(executable.resolve())],capture_output=True,text=True,check=True)
    (args.out/'build.log').write_text(build.stdout+build.stderr)
    rows=cases();request=''.join(f'{t} {v}\n' for row in rows for t,v in row['operations'])
    run=lambda binary:[json.loads(x) for x in subprocess.run([str(binary.resolve())],input=request,capture_output=True,text=True,check=True).stdout.splitlines()]
    expected,actual=run(executable),run(args.binary)
    assert len(expected)==len(actual)==sum(len(row['operations']) for row in rows)
    def normalized(row):
        sums={}
        for at,left,right in row['edges']:
            value=sums.setdefault(at,[0,0]);value[0]+=left;value[1]+=right
        return {**row,'edges':[[at,*value] for at,value in sorted(sums.items()) if any(value)]}
    expected=list(map(normalized,expected));actual=list(map(normalized,actual))
    differences=[{'operation':i,'expected':a,'actual':b} for i,(a,b) in enumerate(zip(expected,actual)) if a!=b]
    digest=lambda file:hashlib.sha256(file.read_bytes()).hexdigest()
    meta={'schema':'mdx.psg-edges.v1','cases':len(rows),'operations':len(expected),'equal':len(expected)-len(differences),
          'source_sha256':digest(source/'psg.c'),'harness_sha256':digest(harness),'binary_sha256':digest(args.binary),
          'limitations':['integrated PSG,100% gain, both channels; logical pulse deltas/phase only','no PCM filtering, mixer, analog or full-System equivalence claim']}
    (args.out/'report.json').write_text(json.dumps({**meta,'differences':differences},indent=2)+'\n')
    if args.fixture:
        cursor=0;fixture=[]
        for row in rows:
            observed=[]
            for master,value in row['operations']:
                observation=expected[cursor];cursor+=1
                if value!=-2:observed.append({'master':master,'write':None if value==-1 else value,**observation})
            fixture.append({**{k:v for k,v in row.items() if k!='operations'},'operations':observed})
        args.fixture.write_text(json.dumps({**{k:v for k,v in meta.items() if k!='binary_sha256'},'cases':fixture},separators=(',',':'))+'\n')
    print(json.dumps(meta));assert not differences


if __name__=='__main__':main()
