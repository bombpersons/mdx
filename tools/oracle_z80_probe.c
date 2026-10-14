/* Executes the unchanged external GPGX Z80 core with authored flat-bus cases. */
#include <stdio.h>
#include <string.h>
#include "z80.h"
static unsigned char memory[65536];
static unsigned int addresses[32], values[32], written;
static unsigned char read_memory(unsigned int address) { return memory[address & 65535]; }
static void write_memory(unsigned int address, unsigned char value) {
  memory[address & 65535] = value;
  if (written < 32) { addresses[written] = address & 65535; values[written++] = value; }
}
static unsigned char read_port(unsigned int port) { return 0xff; }
static void write_port(unsigned int port, unsigned char value) {}
static int interrupt_vector(int line) { return -1; }
int main(void) {
  unsigned int kind, opcode, flags, i, at;
  z80_init(0, interrupt_vector);
  z80_readmem = read_memory; z80_writemem = write_memory;
  z80_readport = read_port; z80_writeport = write_port;
  for (i=0;i<64;i++) z80_readmap[i] = memory + 1024*i;
  while (scanf("%u %u %u", &kind, &opcode, &flags) == 3) {
    for (i=0;i<65536;i++) memory[i] = (unsigned char)((i * 37) ^ (i >> 8) ^ 0x5a);
    memset(&Z80, 0, sizeof(Z80)); Z80.irq_callback = interrupt_vector;
    Z80.pc.w.l=0x1000; Z80.sp.w.l=0x6806; Z80.af.w.l=0xa500|flags;
    Z80.bc.w.l=0x1301; Z80.de.w.l=0x2402; Z80.hl.w.l=0x3503;
    Z80.ix.w.l=0x4604; Z80.iy.w.l=0x5705;
    Z80.af2.w.l=0x9876; Z80.bc2.w.l=0x8765; Z80.de2.w.l=0x7654; Z80.hl2.w.l=0x6543;
    written=0; at=0x1000;
    if (kind==1 || kind==5) memory[at++]=0xdd;
    if (kind==2 || kind==6) memory[at++]=0xfd;
    if (kind==3 || kind==5 || kind==6) memory[at++]=0xcb;
    if (kind==4) memory[at++]=0xed;
    if (kind>=5) memory[at++]=6;
    memory[at++]=opcode; memory[at++]=6; memory[at++]=0x80; memory[at++]=0;
    z80_run(1);
    printf("{\"kind\":%u,\"opcode\":%u,\"flags\":%u,\"cycles\":%u,\"registers\":[%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u],\"writes\":[",
      kind,opcode,flags,Z80.cycles/15,(Z80.pc.w.l + (Z80.halt ? 1 : 0)) & 65535,Z80.af.w.l,Z80.bc.w.l,Z80.de.w.l,Z80.hl.w.l,Z80.ix.w.l,Z80.iy.w.l,Z80.sp.w.l,Z80.af2.w.l,Z80.bc2.w.l,Z80.de2.w.l,Z80.hl2.w.l);
    for(i=0;i<written;i++) printf("%s[%u,%u]",i?",":"",addresses[i],values[i]);
    printf("]}\n");
  }
  return 0;
}
