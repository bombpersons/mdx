/* Private observation of actual FIFO wait decisions; no execution changes. */
extern unsigned int mdx_kb_last_pc, mdx_kb_last_frame;
extern unsigned long mdx_kb_last_ordinal;
static void mdx_kb_fifo_observe(unsigned int data, unsigned int before, unsigned int after)
{
  static FILE *file;
  static int initialized;
  if(!initialized) {
    const char *path=getenv("MDX_KB_FIFO"); initialized=1;
    if(path)file=fopen(path,"wb");
    if(path&&!file)abort();
  }
  if(file) fprintf(file,"{\"ordinal\":%lu,\"pc\":%u,\"frame\":%u,\"before_master\":%u,\"after_master\":%u,\"line_base_master\":%u,\"v_counter\":%u,\"data\":%u,\"code\":%u,\"addr\":%u,\"idx\":%u,\"fifo\":[%u,%u,%u,%u]}\n",mdx_kb_last_ordinal,mdx_kb_last_pc,mdx_kb_last_frame,before,after,mcycles_vdp,v_counter,data,code,addr,fifo_idx,fifo_cycles[0],fifo_cycles[1],fifo_cycles[2],fifo_cycles[3]);
}
