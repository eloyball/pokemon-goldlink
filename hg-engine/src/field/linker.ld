/* Overlay 131 */
OUTPUT_ARCH(arm)
MEMORY {

        rom     : ORIGIN = 0x023C8000, LENGTH = 0x18000
        ewram   : ORIGIN = 0x023C8000, LENGTH = 0x18000
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