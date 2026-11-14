/* Overlay 153 */
/* The wider picture (include/coop_wide.h): code that lives in Overlay 1, in the space of a field
   effect nothing creates (pret asm/overlay_01_021FFC0C.s ov01_021FFCA8..ov01_021FFE98, an unused
   billboard's init/destroy/update/draw, referenced only by the template ov01_02209244, which
   nothing reads). It is linked here at that address; armips/asm/coop_wide.s copies it into
   overlay_0001.bin. The game never loads Overlay 153 itself. LENGTH is that space, so an
   overflow fails the link. No .data or .bss: nothing here keeps state. */
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
                __text_end  = . ;
        } >rom = 0xff

        /DISCARD/ : { *(.data) *(.bss) *(COMMON) *(.comment) *(.ARM.attributes) }
}

ASSERT(CoopOv1_BindMapModel == 0x021FFCA8, "armips/asm/coop_wide.s branches to 0x021FFCA8")
ASSERT(CoopOv1_HidePlaceholder == 0x021FFCAA, "armips/asm/coop_wide.s branches to 0x021FFCAA")
ASSERT((WalkOff_VarGet | 1) == VarGet, "asm/coop_ov1/entry.s: WalkOff_VarGet is not rom.ld's VarGet")
ASSERT((WalkOff_GetMapObjectByID | 1) == GetMapObjectByID, "asm/coop_ov1/entry.s: WalkOff_GetMapObjectByID is not rom.ld's GetMapObjectByID")
ASSERT((WalkOff_DeleteMapObject | 1) == DeleteMapObject, "asm/coop_ov1/entry.s: WalkOff_DeleteMapObject is not rom.ld's DeleteMapObject")
ASSERT((WalkOff_FieldSysGetAttrAddr | 1) == FieldSysGetAttrAddr, "asm/coop_ov1/entry.s: WalkOff_FieldSysGetAttrAddr is not rom.ld's FieldSysGetAttrAddr")
ASSERT((WalkOff_EventObjectMovementMan_Create | 1) == EventObjectMovementMan_Create, "asm/coop_ov1/entry.s: WalkOff_EventObjectMovementMan_Create is not rom.ld's EventObjectMovementMan_Create")
ASSERT((WalkOff__ScheduleObjectEventMovement | 1) == _ScheduleObjectEventMovement, "asm/coop_ov1/entry.s: WalkOff__ScheduleObjectEventMovement is not rom.ld's _ScheduleObjectEventMovement")
ASSERT((WalkOff_ScrCmd_WaitMovement | 1) == ScrCmd_WaitMovement, "asm/coop_ov1/entry.s: WalkOff_ScrCmd_WaitMovement is not rom.ld's ScrCmd_WaitMovement")
