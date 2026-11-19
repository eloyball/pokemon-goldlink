/* Overlay 153 */
/* The wider picture (include/coop_wide.h): code that lives in Overlay 1, in the space of a field
   effect nothing creates (pret asm/overlay_01_021FFC0C.s ov01_021FFCA8..ov01_021FFE98, an unused
   billboard's init/destroy/update/draw, referenced only by the template ov01_02209244, which
   nothing reads). It is linked here at that address; armips/asm/coop_wide.s copies it into
   overlay_0001.bin. The game never loads Overlay 153 itself. LENGTH is that space, so an
   overflow fails the link. The walk-off's movement lists (.bss) are the only state, filled
   while a script runs. */
OUTPUT_ARCH(arm)
MEMORY {

        rom     : ORIGIN = 0x021FFCA8, LENGTH = 0x224
        ewram   : ORIGIN = 0x021FFCA8, LENGTH = 0x224
}

SECTIONS {
        /* The division helpers every overlay gets (asm/thumb_help.s): nothing here divides. */
        /DISCARD/ : { *thumb_help.o(*) }

        .text : {

                FILL (0xABCD)

                __text_start = . ;
                /* The entries armips branches to, at fixed addresses (asm/coop_ov1/entry.s). */
                KEEP(*(.coop_ov1_entry))
                *(.text)
                *(.text.*)
                *(.rodata)
                *(.rodata.*)
                /* The walk-off's movement lists: zeros in the binary, written while a script runs. */
                *(.bss)
                *(.bss.*)
                *(COMMON)
                __text_end  = . ;
        } >rom = 0xff

        /DISCARD/ : { *(.data) *(.comment) *(.ARM.attributes) }
}

ASSERT(CoopOv1_BindMapModel == 0x021FFCA8, "armips/asm/coop_wide.s branches to 0x021FFCA8")
ASSERT(CoopOv1_HidePlaceholder == 0x021FFCAA, "armips/asm/coop_wide.s branches to 0x021FFCAA")
