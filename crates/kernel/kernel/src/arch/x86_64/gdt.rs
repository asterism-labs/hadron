//! The kernel's GDT and TSS.
//!
//! The layout is fixed by SYSCALL and SYSRET. SYSCALL loads CS from
//! `STAR[47:32]` and SS from that value plus 8. SYSRET to 64-bit mode loads
//! SS from `STAR[63:48]` plus 8 and CS from `STAR[63:48]` plus 16. With
//! `STAR[47:32]` = 0x08 and `STAR[63:48]` = 0x18, the slots below line up.
//!
//! | Index | Selector | Entry                                  |
//! |-------|----------|----------------------------------------|
//! | 0     | 0x00     | null                                   |
//! | 1     | 0x08     | kernel code                            |
//! | 2     | 0x10     | kernel data                            |
//! | 3     | 0x18     | empty: 32-bit user code, unused        |
//! | 4     | 0x23     | user data                              |
//! | 5     | 0x2b     | user code                              |
//! | 6, 7  | 0x30     | TSS                                    |
//!
//! Each CPU needs its own GDT and TSS, since `ltr` marks the TSS descriptor
//! busy. Only the boot CPU runs, so there is one of each.

use core::ptr;

use arch_x86_64::{
    Gdt, PrivilegeLevel, SegmentDescriptor, SegmentSelector, SystemDescriptor, TableIndicator,
    TaskStateSegment,
};

use super::tables;
use crate::sync::WriteOnce;

/// Ring 0 code.
pub(crate) const KERNEL_CODE: SegmentSelector = selector(1, PrivilegeLevel::Ring0);
/// Ring 0 data and stack.
pub(crate) const KERNEL_DATA: SegmentSelector = selector(2, PrivilegeLevel::Ring0);
/// Ring 3 data and stack.
pub(crate) const USER_DATA: SegmentSelector = selector(4, PrivilegeLevel::Ring3);
/// Ring 3 code.
pub(crate) const USER_CODE: SegmentSelector = selector(5, PrivilegeLevel::Ring3);
/// The boot CPU's TSS.
pub(crate) const TSS: SegmentSelector = selector(6, PrivilegeLevel::Ring0);

/// Number of 8-byte slots, counting the TSS's two.
const SLOTS: usize = 8;

const fn selector(index: u16, rpl: PrivilegeLevel) -> SegmentSelector {
    SegmentSelector::new(index, TableIndicator::Global, rpl)
}

/// Every entry except the TSS, whose address is known only at run time.
/// The asserts check that each slot gives the selector named above.
const SEGMENTS: Gdt<SLOTS> = {
    let mut gdt = Gdt::new();
    let code = gdt.set_segment(
        KERNEL_CODE.index(),
        SegmentDescriptor::code64(PrivilegeLevel::Ring0),
    );
    let data = gdt.set_segment(
        KERNEL_DATA.index(),
        SegmentDescriptor::data(PrivilegeLevel::Ring0),
    );
    let user_data = gdt.set_segment(
        USER_DATA.index(),
        SegmentDescriptor::data(PrivilegeLevel::Ring3),
    );
    let user_code = gdt.set_segment(
        USER_CODE.index(),
        SegmentDescriptor::code64(PrivilegeLevel::Ring3),
    );
    assert!(code.bits() == KERNEL_CODE.bits() && data.bits() == KERNEL_DATA.bits());
    assert!(user_data.bits() == USER_DATA.bits() && user_code.bits() == USER_CODE.bits());
    gdt
};

/// The boot CPU's TSS. The CPU reads it on stack switches.
static BSP_TSS: WriteOnce<TaskStateSegment> = WriteOnce::new();

/// The boot CPU's GDT. It lives in writable memory because `ltr` writes the
/// TSS descriptor's busy bit.
static BSP_GDT: WriteOnce<Gdt<SLOTS>> = WriteOnce::new();

/// Builds and loads the boot CPU's GDT, reloads the segment registers, and
/// loads the TSS and a null LDT. Replaces the GDT Limine set up, which sits
/// in bootloader-reclaimable memory.
///
/// # Panics
///
/// Panics if called twice.
pub(crate) fn init() {
    let tss = BSP_TSS.init(TaskStateSegment::new());
    let mut gdt = SEGMENTS;
    let tss_descriptor =
        SystemDescriptor::tss(ptr::from_ref(tss) as u64, TaskStateSegment::limit());
    gdt.set_system(TSS.index(), tss_descriptor);
    let pointer = BSP_GDT.init(gdt).pointer();

    // SAFETY: `BSP_GDT` and `BSP_TSS` are statics, so they never move. The
    // table holds the descriptors each selector names, as `SEGMENTS`
    // checks. The reference to the GDT is not used after `pointer`, so the
    // busy bit `ltr` writes does not change memory behind a live reference.
    unsafe {
        tables::lgdt(&pointer);
        tables::load_code_segment(KERNEL_CODE);
        tables::load_data_segments(KERNEL_DATA);
        tables::ltr(TSS);
        tables::lldt(SegmentSelector::NULL);
    }
}

ktest::ktest! {
    fn cpu_uses_the_kernel_gdt() {
        let gdt = BSP_GDT.get().expect("gdt::init ran before the tests");
        assert_eq!(tables::sgdt(), gdt.pointer());
        assert_eq!(tables::code_segment(), KERNEL_CODE);
        assert_eq!(tables::task_register(), TSS);
    }
}

ktest::ktest! {
    fn ltr_marked_the_tss_busy() {
        let gdt = BSP_GDT.get().expect("gdt::init ran before the tests");
        let ty = arch_x86_64::SystemType::of(gdt.slot(TSS.index()));
        assert_eq!(ty, Some(arch_x86_64::SystemType::TssBusy));
    }
}
