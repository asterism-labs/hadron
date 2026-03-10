//! Kernel entry point called by the UEFI boot stub.

use hadron_boot_info::BootInfo;
use hadron_core::addr::VirtAddr;

/// Write a string to COM1 (port 0x3F8) for early boot diagnostics.
fn serial_str(s: &str) {
    for b in s.bytes() {
        unsafe {
            // SAFETY: Port 0x3F8 is the standard COM1 data register. Writing
            // bytes to it is safe during early boot (no contention).
            core::arch::asm!(
                "out dx, al",
                in("dx") 0x3F8u16,
                in("al") b,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

/// Kernel entry point.
///
/// The UEFI boot stub jumps here after setting up page tables, loading
/// the kernel ELF, and building the `BootInfo` struct.
///
/// # Safety
///
/// `boot_info` must point to a valid, fully initialized `BootInfo` struct
/// in memory accessible under the kernel's page tables.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_init(boot_info: *const BootInfo) -> ! {
    // SAFETY: boot_info was set up by the UEFI stub and is in HHDM-mapped memory.
    let bi = unsafe { &*boot_info };

    serial_str("[kernel] entry reached\n");

    // 1. Initialize HHDM offset.
    hadron_mm::hhdm::init(VirtAddr::new_truncate(bi.hhdm_offset));
    serial_str("[kernel] HHDM initialized\n");

    // 2. Register boot services mapper for on-demand HHDM extension.
    // SAFETY: boot_services points to a valid BootServices vtable set up
    // by the UEFI stub. The stub's page tables are still in CR3.
    unsafe { hadron_mm::hhdm::register_boot_mapper(bi.boot_services) };
    serial_str("[kernel] boot mapper registered\n");

    // 3. Initialize PMM with boot-reserved regions.
    // (PMM init will use ensure_mapped to extend HHDM if bitmap memory
    //  is beyond the initial 4 GiB mapping.)
    // TODO: Convert UEFI memory map to PhysMemoryRegion and call pmm::init.
    serial_str("[kernel] PMM init deferred (no memory map conversion yet)\n");

    // 4. Clear boot mapper (would happen after kernel switches to own CR3).
    // For now, clear immediately since we don't build our own page tables yet.
    hadron_mm::hhdm::clear_boot_mapper();
    serial_str("[kernel] boot mapper cleared\n");

    serial_str("[kernel] OK\n");
    loop {
        core::hint::spin_loop();
    }
}
