/* Limine loads the kernel ELF at its link address, which the Limine
   protocol requires to be in the top 2 GiB of the address space. */
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(kmain)

PHDRS
{
    limine_requests PT_LOAD;
    text            PT_LOAD;
    rodata          PT_LOAD;
    data            PT_LOAD;
}

SECTIONS
{
    . = 0xffffffff80000000;

    /* Limine looks for requests between the start and end markers.
       KEEP stops the linker from dropping them, since no code
       references them. */
    .limine_requests : {
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :limine_requests

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .text : { *(.text .text.*) } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .rodata : { *(.rodata .rodata.*) } :rodata

    /* Tests registered with ktest!. The runner reads the array between
       the two symbols. KEEP stops the linker from dropping the entries,
       since no code references them by name. */
    .ktest : {
        __ktest_start = .;
        KEEP(*(.ktest))
        __ktest_end = .;
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : { *(.data .data.*) } :data
    .bss : { *(.bss .bss.*) *(COMMON) } :data

    /DISCARD/ : { *(.eh_frame*) *(.note .note.*) }
}
