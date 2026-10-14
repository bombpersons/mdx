/* Bounded, independent pre-instruction trace of the first DAC command $0D. */
#include <stdio.h>
#include <stdlib.h>
static void kb_z80_observe(void) {
  static FILE *file;static unsigned int init,started,count;
  if(!init){const char *p=getenv("KB_Z80_LOG");init=1;if(p){file=fopen(p,"wb");if(!file)abort();}}
  if(!file || count>=512)return;
  if(PC==0x8fb)started=1;
  if(!started)return;
  fprintf(file,"{\"pc\":%u,\"af\":%u,\"bc\":%u,\"de\":%u,\"hl\":%u,\"ix\":%u,\"iy\":%u,\"sp\":%u}\n",PC,Z80.af.w.l,Z80.bc.w.l,Z80.de.w.l,Z80.hl.w.l,Z80.ix.w.l,Z80.iy.w.l,Z80.sp.w.l);
  if(++count==512){fclose(file);file=NULL;}
}
