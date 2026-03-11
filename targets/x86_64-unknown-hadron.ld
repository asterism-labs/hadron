/* x86_64 Hadron kernel linker script (UEFI boot stub, relocatable PIE)
 *
 * This script produces a higher-half relocatable ELF (ET_DYN) loaded by
 * the UEFI boot stub at KERNEL_VADDR. The ELF must contain a valid
 * PT_DYNAMIC segment so that the stub's ELF loader can process relocations.
 *
 * Section layout:
 *   .text       Executable code
 *   .rodata     Read-only data
 *   .dynsym     Dynamic symbol table  \
 *   .dynstr     Dynamic string table   | required for PT_DYNAMIC
 *   .rela.dyn   Relocations            |
 *   .gnu.hash   Symbol hash table     /
 *   .got        Global offset table (PIC)
 *   .data       Initialized read-write data
 *   .dynamic    Dynamic linking metadata (PT_DYNAMIC segment)
 *   .bss        Uninitialized data (zero-filled)
 */

OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(kernel_init)

KERNEL_VADDR = 0xffffffff80000000;

PHDRS {
    text    PT_LOAD    FLAGS(5);   /* r-x */
    rodata  PT_LOAD    FLAGS(4);   /* r-- */
    data    PT_LOAD    FLAGS(6);   /* rw- */
    dynamic PT_DYNAMIC;            /* dynamic linking metadata */
}

SECTIONS {
    . = KERNEL_VADDR;

    /* ---- Executable segment (r-x) ---- */

    .text : AT(ADDR(.text) - KERNEL_VADDR) {
        __text_start = .;
        *(.text .text.*)
        __text_end = .;
    } :text

    . = ALIGN(4K);

    /* ---- Read-only segment (r--) ---- */

    .rodata : AT(ADDR(.rodata) - KERNEL_VADDR) {
        __rodata_start = .;

        /* Kernel test descriptors (populated only with --cfg ktest) */
        __hadron_kernel_tests_start = .;
        KEEP(*(.hadron_kernel_tests))
        __hadron_kernel_tests_end = .;

        /* Alt-instruction function pointer entries */
        __hadron_alt_fn_start = .;
        KEEP(*(.hadron_alt_fn))
        __hadron_alt_fn_end = .;

        /* Alt-instruction inline patching entries */
        __hadron_alt_instr_start = .;
        KEEP(*(.hadron_alt_instr))
        __hadron_alt_instr_end = .;

        /* Alt-instruction replacement byte sequences */
        __hadron_alt_instr_replacement_start = .;
        KEEP(*(.hadron_alt_instr_replacement))
        __hadron_alt_instr_replacement_end = .;

        /* Log sink descriptors (hadron-log) */
        __hadron_log_sinks_start = .;
        KEEP(*(.hadron_log_sinks))
        __hadron_log_sinks_end = .;

        *(.rodata .rodata.*)
    } :rodata

    /* Dynamic linking tables (read-only, required for valid PT_DYNAMIC) */
    .dynsym : AT(ADDR(.dynsym) - KERNEL_VADDR) { *(.dynsym) } :rodata
    .dynstr : AT(ADDR(.dynstr) - KERNEL_VADDR) { *(.dynstr) } :rodata
    .rela.dyn : AT(ADDR(.rela.dyn) - KERNEL_VADDR) { *(.rela .rela.*) } :rodata
    .gnu.hash : AT(ADDR(.gnu.hash) - KERNEL_VADDR) { *(.gnu.hash) } :rodata
    .hash : AT(ADDR(.hash) - KERNEL_VADDR) { *(.hash) } :rodata

    .hadron_hkif : AT(ADDR(.hadron_hkif) - KERNEL_VADDR) {
        __hadron_hkif_start = .;
        KEEP(*(.hadron_hkif))
        __hadron_hkif_end = .;
        __rodata_end = .;
    } :rodata

    . = ALIGN(4K);

    /* ---- Read-write segment (rw-) ---- */

    .got : AT(ADDR(.got) - KERNEL_VADDR) {
        __data_start = .;
        *(.got .got.*)
    } :data

    .data : AT(ADDR(.data) - KERNEL_VADDR) {
        *(.data .data.*)
    } :data

    /* PT_DYNAMIC segment — required for ET_DYN binaries */
    .dynamic : AT(ADDR(.dynamic) - KERNEL_VADDR) {
        *(.dynamic)
    } :data :dynamic

    /* Per-CPU template section — single instance copied per CPU at boot */
    . = ALIGN(4K);
    .percpu : AT(ADDR(.percpu) - KERNEL_VADDR) {
        __percpu_start = .;
        KEEP(*(.percpu))
        . = ALIGN(64);
        __percpu_end = .;
    } :data

    .bss : AT(ADDR(.bss) - KERNEL_VADDR) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    } :data

    . = ALIGN(4K);
    __data_end = .;    /* covers .got + .data + .dynamic + .bss */
    __kernel_end = .;

    /* Discard sections that are not needed at runtime */
    /DISCARD/ : {
        *(.comment)
        *(.note .note.*)
        *(.eh_frame .eh_frame.*)
    }
}
