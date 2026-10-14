#!/usr/bin/env python3
"""Compare timing costs only in an exact instruction-aligned prefix."""
import collections, json, pathlib, struct, sys

root = pathlib.Path(sys.argv[1])
cpu = (root / 'current.cpu').read_bytes()
mt = (root / 'current.timing').read_bytes()
ot = (root / 'oracle.timing').read_bytes()
end = int(sys.argv[2]) if len(sys.argv)>2 else 768522
hist = collections.Counter()
examples = {}
checkpoints = []
acc = 0
for i in range(end-1):
    pc = struct.unpack_from('<I', cpu, i*72)[0]
    _,mf,line,dot,cycles,dma=struct.unpack_from('<6I',mt,i*24)
    _,of,oc=struct.unpack_from('<3I',ot,i*12)
    _,nf,nc=struct.unpack_from('<3I',ot,(i+1)*12)
    actual = nc-oc+(nf-of)*896040
    ours = cycles*7 + dma-((dma+6)//7)*7
    delta = ours-actual
    acc += delta
    if delta:
        key=(hex(pc),ours,actual,delta)
        hist[key]+=1
        examples.setdefault(key,{'ordinal':i,'mdx':{'frame':mf,'line':line,'dot':dot,'cycles':cycles,'dma':dma},'oracle':{'frame':of,'master':oc},'accumulated_delta':acc})
    if i in [0,6,100,10000,100000,500000,700000,768520]:
        checkpoints.append({'ordinal':i,'accumulated_master_delta':acc,'mdx_frame':mf,'oracle_frame':of})
result={'compared_prefix':end-1,'initial_oracle_master':struct.unpack_from('<3I',ot,0)[2], 'checkpoints':checkpoints,
        'timing_differences':[{'pc':k[0],'mdx_master':k[1],'oracle_master':k[2],'delta':k[3],'count':v,'first':examples[k]} for k,v in hist.most_common()],
        'limitations':['exact ordinal prefix only; oracle frame wrap is 896040 master clocks','cost includes inter-instruction waits; a cost mismatch does not alone locate an intra-instruction bus event']}
(root/'timing-comparison.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
