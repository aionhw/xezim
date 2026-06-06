/* Linker script for the Spike DPI smoke ELF — anchors .text at the
   Spike "main memory" base (0x80000000) so the shim's RAM region holds
   the program after fesvr's load_elf populates it. */

SECTIONS {
    . = 0x80000000;
    .text   : { *(.text*)   }
    .rodata : { *(.rodata*) }
    .data   : { *(.data*)   }
    .bss    : { *(.bss*)    }
}
