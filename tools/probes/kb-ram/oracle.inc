/* Private observation only: pre-dispatch registers and actual bus reads. */
#include <stdio.h>
#include <stdlib.h>
static unsigned int kb_frame = 0xffffffffu, kb_pc, kb_rng_calls, kb_irq_calls;
static unsigned long kb_ordinal;
static FILE *kb_log;
static unsigned char kb_mask[65536];
static unsigned int kb_affected[0x80000];
void retro_mdx_trace_set_frame(unsigned int frame) { kb_frame = frame; }
static void kb_init(void) {
  static int init;
  if (!init) { const char *p=getenv("KB_RAM_LOG"); init=1;
    if(p) {kb_log=fopen(p,"wb"); if(!kb_log) abort(); setvbuf(kb_log,NULL,_IOFBF,65536);}
    p=getenv("KB_RAM_MASK"); if(p){FILE *f=fopen(p,"rb");if(!f||fread(kb_mask,1,65536,f)!=65536)abort();fclose(f);} }
}
static void kb_regs(const char *kind, unsigned int address, unsigned int width, unsigned int value) {
  unsigned int i;
  if(!kb_log) return;
  fprintf(kb_log,"{\"kind\":\"%s\",\"pc\":%u,\"ordinal\":%lu,\"frame\":%u,\"master\":%u,\"address\":%u,\"width\":%u,\"value\":%u,\"sr\":%u,\"rng_call\":%u,\"d\":[",kind,kb_pc,kb_ordinal,kb_frame,m68k.cycles,address,width,value,m68k_get_reg(M68K_REG_SR),kb_rng_calls);
  for(i=0;i<8;i++) fprintf(kb_log,"%s%u",i?",":"",m68k_get_reg((m68k_register_t)(M68K_REG_D0+i)));
  fprintf(kb_log,"],\"a\":[");
  for(i=0;i<8;i++) fprintf(kb_log,"%s%u",i?",":"",m68k_get_reg((m68k_register_t)(M68K_REG_A0+i)));
  fprintf(kb_log,"],\"seed\":[");
  for(i=0;i<6;i++) fprintf(kb_log,"%s%u",i?",":"",READ_BYTE(m68k.memory_map[255].base,0xff46+i));
  fprintf(kb_log,"]}\n");
}
static void kb_dispatch(void) {
  kb_init();kb_pc=m68k_get_reg(M68K_REG_PC);kb_ordinal++;
  if(kb_pc==0x13b0c) kb_rng_calls++;
  if(kb_pc==0x2a060) kb_irq_calls++;
  if(kb_pc==0x111e || kb_pc==0x1122 || kb_pc==0x1124 || kb_pc==0x1128 || kb_pc==0x112c || kb_pc==0x1272 || kb_pc==0x13b0c || kb_pc==0x13bb4 || kb_pc==0x29b86 || (kb_pc==0x2a060 && kb_irq_calls<=12) || (kb_pc>=0x13b0c && kb_pc<0x13bba && kb_rng_calls<=6)) kb_regs("pre",0,0,0);
}
void kb_bus(unsigned int address, unsigned int width, unsigned int value, int writing) {
  unsigned int a=address&0xffffff, off=a&0xffff;
  unsigned int i;
  if (a>=0xe00000 && ((off<=0x8acb && off+width>0x8ac8) || (off<=0xff4b && off+width>0xff46))) kb_regs(writing?"write":"read",address,width,value);
  if (!writing && a==0xc00008) kb_regs("hv",address,width,value);
  if (!writing && off==0xe4ea && (kb_pc==0x13b44 || kb_pc==0x13b78)) kb_regs("tick",address,width,value);
  if (a>=0xe00000 && ((off<0x8b68 && off+width>0x8b00 && kb_frame>=4000) || (off<0x9c32 && off+width>0x9c20 && (kb_pc>=0x29dc8 && kb_pc<=0x29e72)))) kb_regs(writing?"residual_write":"residual_read",address,width,value);
  if (!writing && (a==0xa0000b || a==0xa000da) && kb_pc>=0x29dc8 && kb_pc<=0x29e72) kb_regs("sound_handshake",address,width,value);
  if(!writing && a>=0xe00000 && kb_frame>=4000 && kb_pc<0x80000) {
    for(i=0;i<width;i++) if(kb_mask[(off+i)&65535]) {
      if(kb_affected[kb_pc]++<4) kb_regs("affected_read",address,width,value);
      break;
    }
  }
}
