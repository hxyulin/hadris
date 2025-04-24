ENTRY(_start)

SECTIONS {
    /* The stack is before the 0x7c00 address */
    . = 0x7c00;
   
    .boot : {
        *(.boot)
        *(.boot.*)
    }

    .text : {
        *(.text)
        *(.text.*)
    }

    .rodata : {
        *(.rodata)
        *(.rodata.*)
    }

    .data : {
        *(.data)
        *(.data.*)
    }

    . = 0x7c00 + 446;
    .partition_table : {
        /* Entry 0 */
        QUAD(0)
        QUAD(0)
        /* Entry 1 */
        QUAD(0)
        QUAD(0)
        /* Entry 2 */
        QUAD(0)
        QUAD(0)
        /*Entry 3 */
        QUAD(0)
        QUAD(0)
    }

    . = 0x7c00 + 510;
    .magic_number : {
        /* Magic number */
        SHORT(0xAA55)
    }
    . = 0x7c00 + 512;
}
