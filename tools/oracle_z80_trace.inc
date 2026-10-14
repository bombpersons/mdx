/* Observation only: pre-dispatch Z80 state in a private unchanged-execution core. */
#include <stdio.h>
#include <stdlib.h>
extern unsigned int retro_mdx_trace_get_frame(void);
static void mdx_trace_z80(void)
{
  static FILE *trace;
  static unsigned int written, limit, start, start_frame;
  static int initialized, started;
  if (!initialized) {
    const char *path = getenv("MDX_ORACLE_Z80_TRACE");
    const char *count = getenv("MDX_ORACLE_Z80_LIMIT");
    const char *pc = getenv("MDX_ORACLE_Z80_PC");
    const char *frame = getenv("MDX_ORACLE_Z80_START_FRAME");
    initialized = 1; limit = count ? strtoul(count, NULL, 0) : 512;
    start = pc ? strtoul(pc, NULL, 0) : 0;
    start_frame = frame ? strtoul(frame, NULL, 0) : 0;
    if (path) trace = fopen(path, "w");
    if (path && !trace) { perror("oracle Z80 trace"); abort(); }
  }
  if (!trace || written >= limit || retro_mdx_trace_get_frame() < start_frame) return;
  if (Z80.pc.w.l == start) started = 1;
  if (!started) return;
  fprintf(trace,"{\"frame\":%u,\"pc\":%u,\"cycles\":%u,\"af\":%u,\"bc\":%u,\"de\":%u,\"hl\":%u,\"ix\":%u,\"iy\":%u,\"sp\":%u,\"flag173\":%u}\n",
    retro_mdx_trace_get_frame(), Z80.pc.w.l, Z80.cycles, Z80.af.w.l, Z80.bc.w.l,
    Z80.de.w.l, Z80.hl.w.l, Z80.ix.w.l, Z80.iy.w.l, Z80.sp.w.l,zram[0x173]);
  if (++written == limit) { fclose(trace); trace = NULL; }
}
