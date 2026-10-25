/* Overlay 129 */
OUTPUT_ARCH(arm)
MEMORY {

        rom     : ORIGIN = 0x023D8000 + 0x60, LENGTH = 0x8000 - 0x60
        ewram   : ORIGIN = 0x023D8000 + 0x60, LENGTH = 0x8000 - 0x60
}

SECTIONS {
        .text : {

                FILL (0xABCD)

                __text_start = . ;
                /* Coop mailbox first, so its address (0x023D8060) never moves
                   when code changes. Read by coop-bridge via coop_offsets.json. */
                KEEP(*(.coop_mailbox))
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