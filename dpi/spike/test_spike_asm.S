// Minimal RV32IMC test program for the Spike DPI smoke test.
//
// Pure assembly, no startup code, no stack — each instruction writes a
// known constant into a distinct register. Verifies that the shim
// correctly drives Spike + surfaces processor state across DPI:
//   x1 (ra) = 0xdead
//   x2 (sp) = 0xbeef
//   x3 (gp) = 0x1234
//   x4 (tp) = 0x5678
//   x5 (t0) = 0xcafe
//   x6 (t1) = 0xbabe
//
// Build (Spike-friendly ELF at entry 0x80000000):
//   riscv64-unknown-elf-gcc -march=rv32imc -mabi=ilp32 \
//       -nostartfiles -nostdlib -T link.ld \
//       -o test_spike_asm.elf test_spike_asm.S

.section .text
.globl _start
_start:
    li x1, 0xDEAD
    li x2, 0xBEEF
    li x3, 0x1234
    li x4, 0x5678
    li x5, 0xCAFE
    li x6, 0xBABE
    ebreak
