/* Observation-only hook inserted into a private GPGX build by
 * build_trace_oracle.py. Records pre-instruction PC, SR, D0-D7, A0-A7 as
 * eighteen little-endian u32 values. No target state is changed. */
#include <stdio.h>
#include <stdlib.h>

#ifndef MDX_TRACE_CYCLES
#define MDX_TRACE_CYCLES 0u
#endif
static unsigned int mdx_observer_frame = 0xffffffffu;
#if defined(__GNUC__)
__attribute__((visibility("default")))
#endif
void retro_mdx_trace_set_frame(unsigned int frame) { mdx_observer_frame = frame; }
unsigned int retro_mdx_trace_get_frame(void) { return mdx_observer_frame; }

static void mdx_trace_instruction(void)
{
  static FILE *trace;
  static FILE *timing;
  static unsigned long seen;
  static unsigned long written;
  static unsigned long limit;
  static unsigned long start_pc;
  static unsigned long start_count;
  static int require_pc;
  static int started;
  static int initialized;
  unsigned int values[18];
  unsigned char record[72];
  unsigned int i;
  if (!initialized)
  {
    const char *path = getenv("MDX_ORACLE_TRACE");
    const char *maximum = getenv("MDX_ORACLE_TRACE_LIMIT");
    const char *start = getenv("MDX_ORACLE_TRACE_START_PC");
    const char *skip = getenv("MDX_ORACLE_TRACE_START_COUNT");
    const char *timing_path = getenv("MDX_ORACLE_TRACE_TIMING");
    initialized = 1;
    limit = maximum ? strtoul(maximum, NULL, 10) : 1000000;
    start_pc = start ? strtoul(start, NULL, 0) : 0;
    start_count = skip ? strtoul(skip, NULL, 0) : 0;
    require_pc = start != NULL;
    if (path) trace = fopen(path, "wb");
    if (timing_path) timing = fopen(timing_path, "wb");
    if (path && !trace) { perror("oracle trace"); abort(); }
    if (timing_path && !timing) { perror("oracle timing trace"); abort(); }
  }
  if (!trace || written >= limit) return;
  values[0] = m68k_get_reg(M68K_REG_PC);
  if (!started && seen >= start_count && (!require_pc || values[0] == start_pc)) started = 1;
  ++seen;
  if (!started) return;
  values[1] = m68k_get_reg(M68K_REG_SR);
  for (i = 0; i < 16; ++i)
    values[i + 2] = m68k_get_reg((m68k_register_t)(M68K_REG_D0 + i));
  for (i = 0; i < 18; ++i)
  {
    record[i*4] = values[i];
    record[i*4+1] = values[i] >> 8;
    record[i*4+2] = values[i] >> 16;
    record[i*4+3] = values[i] >> 24;
  }
  if (fwrite(record, 1, sizeof(record), trace) != sizeof(record)) abort();
  if (timing) {
    unsigned int metadata[3] = {(unsigned int)(seen-1),mdx_observer_frame,MDX_TRACE_CYCLES};
    unsigned char bytes[12];
    for (i=0;i<3;++i) { bytes[i*4]=metadata[i];bytes[i*4+1]=metadata[i]>>8;
      bytes[i*4+2]=metadata[i]>>16;bytes[i*4+3]=metadata[i]>>24; }
    if (fwrite(bytes,1,sizeof(bytes),timing)!=sizeof(bytes)) abort();
  }
  if (++written == limit) { fclose(trace); trace = NULL;if(timing){fclose(timing);timing=NULL;} }
}
