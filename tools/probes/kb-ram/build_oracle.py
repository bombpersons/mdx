#!/usr/bin/env python3
"""Copy clean cached GPGX and add observation-only hooks, never alter the cache."""
import hashlib,json,pathlib,shutil,subprocess
root=pathlib.Path(__file__).resolve().parents[3]
dest=root/'.mdx/residual-review/kb/oracle-build-v7'
source=root/'cache/oracle/Genesis-Plus-GX'
if dest.exists():raise SystemExit('output must be absent')
shutil.copytree(source,dest,ignore=shutil.ignore_patterns('.git','*.o','*.so'))
p=dest/'core/m68k/m68kcpu.c';text=p.read_text();before=hashlib.sha256(p.read_bytes()).hexdigest()
anchor='REG_IR = m68ki_read_imm_16();'
assert text.count(anchor)==2
text=text.replace('#include "m68kops.h"','#include "m68kops.h"\n'+(root/'tools/probes/kb-ram/oracle.inc').read_text(),1)
text=text.replace(anchor,'kb_dispatch();\n      '+anchor)
p.write_text(text)
h=dest/'core/m68k/m68kcpu.h';text=h.read_text()
text=text.replace('INLINE uint m68ki_read_8(uint address)\n{','extern void kb_bus(unsigned int, unsigned int, unsigned int, int);\nINLINE uint m68ki_read_8(uint address)\n{',1)
for width in (8,16,32):
 start=text.index(f'INLINE uint m68ki_read_{width}(uint address)\n{{');end=text.index('\n}',start)
 part=text[start:end];assert part.count('return val;')==1
 text=text[:start]+part.replace('return val;',f'kb_bus(address,{width//8},val,0);\n  return val;')+text[end:]
 start=text.index(f'INLINE void m68ki_write_{width}(uint address, uint value)\n{{');end=text.index('\n}',start)
 part=text[start:end];pos=part.index('  temp = &m68ki_cpu.memory_map')
 part=part[:pos]+f'  kb_bus(address,{width//8},value,1);\n'+part[pos:]
 text=text[:start]+part+text[end:]
h.write_text(text)
z=dest/'core/z80/z80.c';text=z.read_text();anchor='void z80_run(unsigned int cycles)'
assert text.count(anchor)==1
text=text.replace(anchor,(root/'tools/probes/kb-ram/oracle_z80.inc').read_text()+'\n'+anchor)
anchor='    Z80.after_ei = FALSE;';assert text.count(anchor)==1
z.write_text(text.replace(anchor,'    kb_z80_observe();\n'+anchor))
subprocess.run(['make','-f','Makefile.libretro','-j2'],cwd=dest,check=True)
(dest/'observation-manifest.json').write_text(json.dumps({'source_cpu_sha256':before,'hook_sha256':hashlib.sha256((root/'tools/probes/kb-ram/oracle.inc').read_bytes()).hexdigest(),'core_sha256':hashlib.sha256((dest/'genesis_plus_gx_libretro.so').read_bytes()).hexdigest(),'scope':'pre-instruction and actual selected data bus access observation; no emulated state mutations'},indent=2)+'\n')
