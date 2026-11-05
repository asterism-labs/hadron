//! Limine requests and the kernel entry point.

use limine::{BaseRevision, BootloaderInfoRequest, RequestsEndMarker, RequestsStartMarker};

use crate::arch::x86_64::qemu::{self, ExitCode};
use crate::arch::x86_64::{gdt, idt};
use crate::console::{self, kprintln};
use crate::memory;
use crate::percpu;

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static BOOTLOADER_INFO: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

/// Entry point. Limine jumps here, as named by `ENTRY` in `kernel.ld`.
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    // Locks mask interrupts through the per-CPU block, so it comes first.
    percpu::init_boot_cpu();
    // With no console there is nowhere to report the error, so boot goes on.
    let _ = console::init();
    gdt::init();
    idt::init();

    if !BASE_REVISION.is_supported() {
        kprintln!(
            "hadron: Limine does not support base revision {}",
            BASE_REVISION.revision_slot()
        );
        qemu::exit(ExitCode::Failure);
    }
    let (name, version) = BOOTLOADER_INFO
        .response()
        .map_or(("an unknown bootloader", ""), |info| {
            (info.name(), info.version())
        });
    kprintln!("hadron: booted by {name} {version}");
    if let Some(revision) = BASE_REVISION.loaded_revision() {
        kprintln!("hadron: Limine base revision {revision}");
    }
    memory::init(kmain_paged)
}

/// Runs on the kernel's own page tables and boot stack, after
/// [`memory::init`].
fn kmain_paged() -> ! {
    const MIB: u64 = 1 << 20;
    crate::heap::init();
    let usable = memory::map().total(mm::RegionKind::Usable);
    let (free, count) = {
        let frames = memory::frames();
        (frames.free_frames(), frames.frame_count())
    };
    kprintln!(
        "hadron: {} MiB usable, {free} of {count} frames free",
        usable / MIB
    );
    kprintln!("hadron: {} KiB heap", crate::heap::free_bytes() / 1024);
    if hadron_config::KTEST {
        crate::ktest::run();
    }
    qemu::exit(ExitCode::Success)
}

ktest::ktest! {
    fn limine_answers_bootloader_info() {
        assert!(BOOTLOADER_INFO.response().is_some());
    }
}
