/* Observation-only, private KB event hook. Original dispatch semantics unchanged. */
unsigned int mdx_kb_last_pc;
unsigned long mdx_kb_last_ordinal;
unsigned int mdx_kb_last_frame;
static void mdx_kb_observe(void)
{
  static FILE *file;
  static unsigned long ordinal;
  static int initialized;
  unsigned int pc = m68k_get_reg(M68K_REG_PC);
  unsigned int i;
  mdx_kb_last_pc=pc;mdx_kb_last_ordinal=ordinal;mdx_kb_last_frame=mdx_observer_frame;
  if (!initialized) {
    const char *path = getenv("MDX_KB_EVENTS");
    initialized=1;
    if (path) file=fopen(path,"wb");
    if (path && !file) abort();
  }
  if (file && (pc == 0x16e0 || pc == 0x16b0 || pc == 0x2a060)) {
    fprintf(file,"{\"ordinal\":%lu,\"pc\":%u,\"frame\":%u,\"master\":%u,\"refresh_at\":%u,\"sr\":%u,\"d\":[",ordinal,pc,mdx_observer_frame,m68k.cycles,m68k.refresh_cycles,m68k_get_reg(M68K_REG_SR));
    for(i=0;i<8;i++) fprintf(file,"%s%u",i?",":"",m68k_get_reg((m68k_register_t)(M68K_REG_D0+i)));
    fprintf(file,"],\"a\":[");
    for(i=0;i<8;i++) fprintf(file,"%s%u",i?",":"",m68k_get_reg((m68k_register_t)(M68K_REG_A0+i)));
    fprintf(file,"]}\n");
  }
  ordinal++;
}
