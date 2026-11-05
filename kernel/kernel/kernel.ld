/* Limine loads the kernel ELF at its link address, which the Limine
   protocol requires to be in the top 2 GiB of the address space. */
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(kmain)

PHDRS
{
    text            PT_LOAD;
    rodata          PT_LOAD;
    data            PT_LOAD;
}

SECTIONS
{
    . = 0xffffffff80000000;
    __kernel_start = .;

    /* The kernel maps each part with its own permissions, using the
       __*_start and __*_end symbols. Each part starts on a page. */
    .text : {
        __text_start = .;
        *(.text .text.*)
        __text_end = .;
    } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .rodata : {
        __rodata_start = .;
        *(.rodata .rodata.*)
    } :rodata

    /* Tests registered with ktest!. The runner reads the array between
       the two symbols. KEEP stops the linker from dropping the entries,
       since no code references them by name. */
    .ktest : {
        __ktest_start = .;
        KEEP(*(.ktest))
        __ktest_end = .;
        __rodata_end = .;
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : {
        __data_start = .;
        *(.data .data.*)
    } :data
    /* The libraries are built position independent, so they reach
       statics through the GOT. Placed here, the kernel maps it with
       the data. Left to the linker, it lands outside every part. */
    .got : {
        *(.got .got.*)
    } :data
    .bss : {
        *(.bss .bss.*) *(COMMON)
        __data_end = .;
    } :data

    /DISCARD/ : { *(.eh_frame*) *(.note .note.*) }
}
