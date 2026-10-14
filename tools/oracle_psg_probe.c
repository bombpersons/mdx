/* Authored observer harness for separately obtained, unchanged GPGX psg.c. */
#include "shared.h"
#include <stdio.h>
#include <stdlib.h>
mdx_sound snd;
mdx_config config;
int8 audio_hard_disable;
static unsigned times[65536], count;
static int left[65536], right[65536];
void blip_add_delta(blip_t *buffer, unsigned time, int l, int r)
{
  (void)buffer;
  if (!l && !r) return;
  if (count == 65536) abort();
  times[count]=time;left[count]=l;right[count++]=r;
}
void blip_add_delta_fast(blip_t *buffer, unsigned time, int l, int r)
{ blip_add_delta(buffer,time,l,r); }
int main(void)
{
  unsigned clock; int value;
  psg_init(PSG_INTEGRATED);psg_reset();
  while (scanf("%u %d",&clock,&value)==2) {
    int state[31]; unsigned i;
    count=0;
    if (value==-2) psg_reset();
    else if (value==-1) psg_config(clock,100,255);
    else psg_write(clock,(unsigned)value);
    if (psg_context_save((uint8*)state)!=sizeof(state)) abort();
    printf("{\"edges\":[");
    for(i=0;i<count;i++) printf("%s[%u,%d,%d]",i?",":"",times[i],left[i],right[i]);
    printf("],\"phase\":{\"clock\":%d,\"latch\":%d,\"noise\":%d,",state[0],state[1],state[2]);
    printf("\"periods\":[%d,%d,%d,%d],",state[11],state[12],state[13],state[14]);
    printf("\"next\":[%d,%d,%d,%d],",state[15],state[16],state[17],state[18]);
    printf("\"polarity\":[%d,%d,%d,%d],",state[19],state[20],state[21],state[22]);
    printf("\"amplitude\":[%d,%d,%d,%d]}}\n",state[23],state[25],state[27],state[29]);
  }
  return 0;
}
