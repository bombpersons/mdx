/* Observation-only wrapper around the original Z80/68000 bus request. */
#include <stdio.h>
#include <stdlib.h>
extern unsigned int retro_mdx_trace_get_frame(void);
static void z80_request_68k_bus_access(void)
{
  static FILE *trace;
  static int initialized;
  static unsigned int written;
  static unsigned int start_frame;
  unsigned int before68k = m68k.cycles;
  unsigned int beforez80 = Z80.cycles;
  z80_request_68k_bus_access_unobserved();
  if (!initialized) {
    const char *path = getenv("MDX_ORACLE_BUS_TRACE");
    const char *start = getenv("MDX_ORACLE_BUS_START_FRAME");
    start_frame = start ? strtoul(start, NULL, 0) : 0;
    initialized = 1;
    if (path) trace = fopen(path, "w");
    if (path && !trace) { perror("oracle bus trace"); abort(); }
  }
  if (trace && written < 4096 && retro_mdx_trace_get_frame() >= start_frame) {
    fprintf(trace, "{\"frame\":%u,\"cpu_pc\":%u,\"z80_pc\":%u,\"cpu_before\":%u,\"cpu_after\":%u,\"z80_before\":%u,\"z80_after\":%u,\"line_start\":%u}\n",
      retro_mdx_trace_get_frame(), m68k_get_reg(M68K_REG_PC), Z80.pc.w.l,
      before68k, m68k.cycles, beforez80, Z80.cycles, mcycles_vdp);
    if (++written == 4096) { fclose(trace); trace = NULL; }
  }
}
