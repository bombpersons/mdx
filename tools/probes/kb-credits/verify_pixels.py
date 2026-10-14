#!/usr/bin/env python3
"""Run the seven existing unchanged KB frame/input/reference gates in a private probe."""
import hashlib, json, pathlib, subprocess, sys
from PIL import Image
root=pathlib.Path(__file__).resolve().parents[3]
out=root/'.mdx/fidelity-review/kb-investigation/gates'
out.mkdir(exist_ok=True)
mode=sys.argv[1] if len(sys.argv)>1 else 'fifo-refresh-quiet'
cases=[(400,'frame400',None),(1200,'frame1200',None),(3600,'character3600','kb_character_replay.json'),(3900,'difficulty3900','kb_new_game_replay.json'),(4300,'world4300','kb_world_replay.json'),(4500,'move4500','kb_move_replay.json'),(4500,'walk4500','kb_walk_replay.json')]
results=[]
for frame,name,replay in cases:
    prefix=out/(mode+'-'+name)
    command=[str(root/'tools/probes/kb-credits/target/release/kb-credits-probe'),str(root/'rom/original/kings_bounty.md'),'100000000',str(prefix),mode,str(frame)]
    if replay:command.append(str(root/'testdata'/replay))
    p=subprocess.run(command,check=True,capture_output=True,text=True)
    pathlib.Path(str(prefix)+'.log').write_text(p.stdout)
    reference=root/'testdata'/('oracle_kb_'+name+'_rgb.png')
    a=pathlib.Path(str(prefix)+'.rgb').read_bytes();b=Image.open(reference).convert('RGB').tobytes()
    changed=sum(a[i:i+3]!=b[i:i+3] for i in range(0,len(a),3))
    result={'frame':frame,'case':name,'replay':replay,'replay_sha256':hashlib.sha256((root/'testdata'/replay).read_bytes()).hexdigest() if replay else None,'reference':str(reference),'reference_sha256':hashlib.sha256(reference.read_bytes()).hexdigest(),'changed_pixels':changed,'execution':json.loads(p.stdout)}
    results.append(result); print(json.dumps(result),flush=True)
(out/(mode+'-results.json')).write_text(json.dumps(results,indent=2)+'\n')
sys.exit(any(x['changed_pixels'] for x in results))
