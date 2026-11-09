/* Overlay 130 */
OUTPUT_ARCH(arm)
MEMORY {

        rom     : ORIGIN = 0x023C4000, LENGTH = 0x14000
        ewram   : ORIGIN = 0x023C4000, LENGTH = 0x14000
}

SECTIONS {
        .text : {

                FILL (0xABCD)

                __text_start = . ;
                /* The co-op header first, at the address resident code calls
                   through (include/coop_link.h COOP_BATTLE_EXT). */
                KEEP(*(.coop_battle_ext))
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

ASSERT(gCoopBattleExt == 0x023C4000, "the co-op battle header must sit at 0x023C4000 (include/coop_link.h)")
