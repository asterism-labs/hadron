//! The kernel's IDT. Every vector enters through an interrupt gate, so
//! handlers run with interrupts masked. Double faults switch to their own
//! stack.

use arch_x86_64::{Exception, GateDescriptor, GateType, Idt, PrivilegeLevel};

use super::{gdt, tables, trap};
use crate::sync::WriteOnce;

/// The IDT. All CPUs can share it.
static IDT: WriteOnce<Idt> = WriteOnce::new();

/// Builds and loads the IDT. Call after [`gdt::init`], since the gates
/// name the kernel code segment and the double fault stack.
///
/// # Panics
///
/// Panics if called twice.
pub(crate) fn init() {
    let mut idt = Idt::new();
    for (vector, &stub) in (0..=u8::MAX).zip(trap::stubs()) {
        let ist = (vector == Exception::DoubleFault as u8).then_some(gdt::DOUBLE_FAULT_IST);
        let gate = GateDescriptor::new(
            stub,
            gdt::KERNEL_CODE,
            GateType::Interrupt,
            PrivilegeLevel::Ring0,
            ist,
        );
        idt.set(vector, gate);
    }
    let pointer = IDT.init(idt).pointer();
    // SAFETY: `IDT` is a static, so it never moves. Each gate enters a stub
    // from `trap`, through the kernel code segment `gdt::init` loaded.
    unsafe { tables::lidt(&pointer) };
}

ktest::ktest! {
    fn cpu_uses_the_kernel_idt() {
        let idt = IDT.get().expect("idt::init ran before the tests");
        assert_eq!(tables::sidt(), idt.pointer());
        let double_fault = idt.gate(Exception::DoubleFault as u8);
        assert_eq!(double_fault.ist(), Some(gdt::DOUBLE_FAULT_IST));
        assert_eq!(idt.gate(Exception::PageFault as u8).ist(), None);
        assert!(idt.gate(32).is_present());
        assert!(idt.gate(255).is_present());
    }
}

ktest::ktest! {
    fn breakpoint_returns_to_the_caller() {
        let before = trap::breakpoints();
        super::cpu::breakpoint();
        super::cpu::breakpoint();
        assert_eq!(trap::breakpoints(), before + 2);
    }
}

ktest::ktest! {
    fn breakpoint_preserves_registers() {
        // Load a distinct value into every register Rust lets inline
        // assembly name, take a breakpoint, and read them back. RBX and RBP
        // are reserved by LLVM, but the entry stub saves them the same way.
        let mut regs: [u64; 13] = core::array::from_fn(|i| 0x1111_1111_1111_1111 * (i as u64 + 1));
        let expected = regs;
        // SAFETY: `int3` enters the #BP handler, which returns with every
        // register restored. Each register is an in-out operand.
        unsafe {
            core::arch::asm!(
                "int3",
                inout("rax") regs[0], inout("rcx") regs[1], inout("rdx") regs[2],
                inout("rsi") regs[3], inout("rdi") regs[4], inout("r8") regs[5],
                inout("r9") regs[6], inout("r10") regs[7], inout("r11") regs[8],
                inout("r12") regs[9], inout("r13") regs[10], inout("r14") regs[11],
                inout("r15") regs[12],
            );
        }
        assert_eq!(regs, expected);
    }
}
