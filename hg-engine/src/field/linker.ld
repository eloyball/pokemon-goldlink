/* Overlay 131 */
/* 0x023C4000..0x023D8000, the battle extension's range (src/battle/linker.ld): the two are never
   loaded together. LENGTH ends where the resident Overlay 129 starts (src/linker.ld), so an
   overflow fails the link. The lowest 0x3C00 below it are hg-engine's individual overlays
   (src/individual/linker/, 0x023C0400). Milestone 5 Phase 3 moved it down from 0x023C8000. */
OUTPUT_ARCH(arm)
MEMORY {

        rom     : ORIGIN = 0x023C4000, LENGTH = 0x14000
        ewram   : ORIGIN = 0x023C4000, LENGTH = 0x14000
}

SECTIONS {
        .text : {

                FILL (0xABCD)

                __text_start = . ;
                /* Coop field entry header first, so Overlay 129 can call into
                   this overlay at a fixed address (COOP_FIELD_ENTRY_ADDR). */
                KEEP(*(.coop_field_entry))
                *(.init)
                *(.text)
                *(.ctors)
                *(.dtors)
                *(.rodata)
                *(.fini)
                *(COMMON)
                __text_end  = . ;

                __bss_start__ = . ;
                *(.bss)
                __bss_end__ = . ;
        _end = __bss_end__ ;
        __end__ = __bss_end__ ;
        } >rom = 0xff
}

ASSERT(gCoopFieldEntryHeader == 0x023C4000, "the co-op field header must sit at COOP_FIELD_ENTRY_ADDR (include/coop_sync.h)")