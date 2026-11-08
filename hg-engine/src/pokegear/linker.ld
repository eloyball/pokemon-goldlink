/* Overlay 152 */
/* Milestone 6 §6.2: players on the Pokegear map. Loaded with the vanilla Overlay 101 (the
   Pokegear's apps and the Fly map) through gLinkedOverlayList (src/overlay.c). 0x023C4000 is the
   field and battle extensions' range (src/field/linker.ld, src/battle/linker.ld): an app runs
   without the field or a battle, so neither is loaded. LENGTH ends where the resident Overlay 129
   starts, so an overflow fails the link. Its .bss is never zeroed: state lives in .data. */
OUTPUT_ARCH(arm)
MEMORY {

        rom     : ORIGIN = 0x023C4000, LENGTH = 0x14000
        ewram   : ORIGIN = 0x023C4000, LENGTH = 0x14000
}

SECTIONS {
        .text : {

                FILL (0xABCD)

                __text_start = . ;
                /* The state block first, at a fixed address the probes read. */
                KEEP(*(.coop_pgmap_state))
                *(.init)
                *(.text)
                *(.ctors)
                *(.dtors)
                *(.rodata)
                *(.data)
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

ASSERT(gCoopPgMap == 0x023C4000, "the Pokegear map state must sit at 0x023C4000 (dev probes read it)")
