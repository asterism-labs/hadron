/* x86_64 Hadron kernel linker script (Limine bootloader, relocatable PIE)
 *
 * This script produces a higher-half relocatable ELF (ET_DYN) loaded by
 * Limine at KERNEL_VADDR.  The ELF must contain a valid PT_DYNAMIC segment
 * so that Limine's ELF loader accepts it.
 *
 * Section layout:
 *   .text       Executable code
 *   .rodata     Read-only data (includes Limine request markers)
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
ENTRY(_start)
EXTERN(__HADRON_DRIVERS_ANCHOR)

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
        /* Limine request markers — bootloader scans for magic bytes between
         * the start and end markers. No special section name required. */
        KEEP(*(.requests_start))
        KEEP(*(.requests))
        KEEP(*(.requests_end))

        /* Driver registration tables — kernel scans these during init */
        __hadron_pci_drivers_start = .;
        KEEP(*(.hadron_pci_drivers))
        __hadron_pci_drivers_end = .;

        __hadron_platform_drivers_start = .;
        KEEP(*(.hadron_platform_drivers))
        __hadron_platform_drivers_end = .;

        /* Filesystem driver entries */
        __hadron_block_fs_start = .;
        KEEP(*(.hadron_block_fs))
        __hadron_block_fs_end = .;

        __hadron_virtual_fs_start = .;
        KEEP(*(.hadron_virtual_fs))
        __hadron_virtual_fs_end = .;

        __hadron_initramfs_start = .;
        KEEP(*(.hadron_initramfs))
        __hadron_initramfs_end = .;

        *(.rodata .rodata.*)
    } :rodata

    /* Dynamic linking tables (read-only, required for valid PT_DYNAMIC) */
    .dynsym : AT(ADDR(.dynsym) - KERNEL_VADDR) { *(.dynsym) } :rodata
    .dynstr : AT(ADDR(.dynstr) - KERNEL_VADDR) { *(.dynstr) } :rodata
    .rela.dyn : AT(ADDR(.rela.dyn) - KERNEL_VADDR) { *(.rela .rela.*) } :rodata
    .gnu.hash : AT(ADDR(.gnu.hash) - KERNEL_VADDR) { *(.gnu.hash) } :rodata
    .hash : AT(ADDR(.hash) - KERNEL_VADDR) { *(.hash) __rodata_end = .; } :rodata

    . = ALIGN(4K);

    /* ---- Read-write segment (rw-) ---- */

    .got : AT(ADDR(.got) - KERNEL_VADDR) {
        __data_start = .;
        *(.got .got.*)
    } :data

    .data : AT(ADDR(.data) - KERNEL_VADDR) {
        *(.data .data.*)
    } :data

    /* PT_DYNAMIC segment — Limine requires this for ET_DYN binaries */
    .dynamic : AT(ADDR(.dynamic) - KERNEL_VADDR) {
        *(.dynamic)
    } :data :dynamic

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
