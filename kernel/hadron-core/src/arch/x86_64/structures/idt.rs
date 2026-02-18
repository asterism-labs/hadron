//! Interrupt Descriptor Table (IDT) structures.

use core::ops::{Index, IndexMut};

use crate::addr::VirtAddr;
use crate::arch::x86_64::instructions::segmentation;
use crate::arch::x86_64::structures::gdt::DescriptorTablePointer;

/// Handler function for interrupts without an error code.
pub type HandlerFunc = extern "x86-interrupt" fn(InterruptStackFrame);

/// Handler function for interrupts that push an error code.
pub type HandlerFuncWithErrCode = extern "x86-interrupt" fn(InterruptStackFrame, u64);

/// Diverging handler function for interrupts without an error code (e.g., machine check).
pub type DivergingHandlerFunc = extern "x86-interrupt" fn(InterruptStackFrame) -> !;

/// Diverging handler function for interrupts with an error code (e.g., double fault).
pub type DivergingHandlerFuncWithErrCode = extern "x86-interrupt" fn(InterruptStackFrame, u64) -> !;

/// The stack frame pushed by the CPU when an interrupt occurs.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct InterruptStackFrame {
    /// Instruction pointer at the time of the interrupt.
    pub instruction_pointer: VirtAddr,
    /// Code segment selector.
    pub code_segment: u64,
    /// CPU flags (RFLAGS).
    pub cpu_flags: u64,
    /// Stack pointer at the time of the interrupt.
    pub stack_pointer: VirtAddr,
    /// Stack segment selector.
    pub stack_segment: u64,
}

/// Options for an IDT entry (stored in bits 32..47 of the entry).
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct EntryOptions(u16);

impl EntryOptions {
    /// Creates a minimal entry options value (interrupt gate, not present, DPL=0, IST=0).
    #[inline]
    const fn minimal() -> Self {
        // Type = 0xE (64-bit interrupt gate), present = 0
        Self(0x0E00)
    }

    /// Creates entry options for a present interrupt gate (DPL=0, IST=0).
    #[inline]
    fn new() -> Self {
        let mut opts = Self::minimal();
        opts.set_present(true);
        opts
    }

    /// Sets the IST index (0 = no IST, 1-7 = IST1-IST7).
    #[inline]
    pub fn set_ist_index(&mut self, index: u8) -> &mut Self {
        debug_assert!(index < 8, "IST index must be 0-7");
        self.0 = (self.0 & !0x07) | (index as u16 & 0x07);
        self
    }

    /// Sets the descriptor privilege level (0-3).
    #[inline]
    pub fn set_dpl(&mut self, dpl: u8) -> &mut Self {
        debug_assert!(dpl < 4, "DPL must be 0-3");
        self.0 = (self.0 & !0x6000) | ((dpl as u16 & 0x03) << 13);
        self
    }

    /// Sets the present bit.
    #[inline]
    pub fn set_present(&mut self, present: bool) -> &mut Self {
        if present {
            self.0 |= 1 << 15;
        } else {
            self.0 &= !(1 << 15);
        }
        self
    }

    /// Sets the gate type to trap gate (0xF) instead of interrupt gate (0xE).
    ///
    /// Trap gates do not clear the IF flag on entry.
    #[inline]
    pub fn set_trap_gate(&mut self) -> &mut Self {
        self.0 |= 1 << 8;
        self
    }
}

/// A single IDT entry (16 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct IdtEntry {
    offset_low: u16,
    segment_selector: u16,
    options: EntryOptions,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

impl IdtEntry {
    /// Creates a not-present IDT entry.
    pub const fn missing() -> Self {
        Self {
            offset_low: 0,
            segment_selector: 0,
            options: EntryOptions::minimal(),
            offset_mid: 0,
            offset_high: 0,
            _reserved: 0,
        }
    }

    /// Sets a handler function (no error code) and returns a mutable reference
    /// to the entry options for further configuration.
    pub fn set_handler(&mut self, handler: HandlerFunc) -> &mut EntryOptions {
        self.set_handler_addr(handler as u64)
    }

    /// Sets a handler function with error code and returns a mutable reference
    /// to the entry options.
    pub fn set_handler_with_err_code(
        &mut self,
        handler: HandlerFuncWithErrCode,
    ) -> &mut EntryOptions {
        self.set_handler_addr(handler as u64)
    }

    /// Sets a diverging handler function (no error code).
    pub fn set_diverging_handler(&mut self, handler: DivergingHandlerFunc) -> &mut EntryOptions {
        self.set_handler_addr(handler as u64)
    }

    /// Sets a diverging handler function with error code.
    pub fn set_diverging_handler_with_err_code(
        &mut self,
        handler: DivergingHandlerFuncWithErrCode,
    ) -> &mut EntryOptions {
        self.set_handler_addr(handler as u64)
    }

    fn set_handler_addr(&mut self, addr: u64) -> &mut EntryOptions {
        self.offset_low = addr as u16;
        self.offset_mid = (addr >> 16) as u16;
        self.offset_high = (addr >> 32) as u32;

        self.segment_selector = segmentation::read_cs().as_u16();

        self.options = EntryOptions::new();
        &mut self.options
    }
}

/// The Interrupt Descriptor Table with named fields for all 32 CPU exceptions.
#[repr(C, align(16))]
pub struct InterruptDescriptorTable {
    /// Vector 0: Divide Error (#DE).
    pub divide_error: IdtEntry,
    /// Vector 1: Debug (#DB).
    pub debug: IdtEntry,
    /// Vector 2: Non-Maskable Interrupt (NMI).
    pub nmi: IdtEntry,
    /// Vector 3: Breakpoint (#BP).
    pub breakpoint: IdtEntry,
    /// Vector 4: Overflow (#OF).
    pub overflow: IdtEntry,
    /// Vector 5: Bound Range Exceeded (#BR).
    pub bound_range: IdtEntry,
    /// Vector 6: Invalid Opcode (#UD).
    pub invalid_opcode: IdtEntry,
    /// Vector 7: Device Not Available (#NM).
    pub device_not_available: IdtEntry,
    /// Vector 8: Double Fault (#DF) — always pushes error code 0.
    pub double_fault: IdtEntry,
    /// Vector 9: Reserved (Coprocessor Segment Overrun, legacy).
    _reserved_9: IdtEntry,
    /// Vector 10: Invalid TSS (#TS).
    pub invalid_tss: IdtEntry,
    /// Vector 11: Segment Not Present (#NP).
    pub segment_not_present: IdtEntry,
    /// Vector 12: Stack-Segment Fault (#SS).
    pub stack_segment_fault: IdtEntry,
    /// Vector 13: General Protection (#GP).
    pub general_protection: IdtEntry,
    /// Vector 14: Page Fault (#PF).
    pub page_fault: IdtEntry,
    /// Vector 15: Reserved.
    _reserved_15: IdtEntry,
    /// Vector 16: x87 Floating-Point Exception (#MF).
    pub x87_floating_point: IdtEntry,
    /// Vector 17: Alignment Check (#AC).
    pub alignment_check: IdtEntry,
    /// Vector 18: Machine Check (#MC).
    pub machine_check: IdtEntry,
    /// Vector 19: SIMD Floating-Point Exception (#XM).
    pub simd_floating_point: IdtEntry,
    /// Vector 20: Virtualization Exception (#VE).
    pub virtualization: IdtEntry,
    /// Vector 21: Control Protection Exception (#CP).
    pub control_protection: IdtEntry,
    /// Vectors 22-27: Reserved.
    _reserved_22_27: [IdtEntry; 6],
    /// Vector 28: Hypervisor Injection Exception (#HV).
    pub hypervisor_injection: IdtEntry,
    /// Vector 29: VMM Communication Exception (#VC).
    pub vmm_communication: IdtEntry,
    /// Vector 30: Security Exception (#SX).
    pub security_exception: IdtEntry,
    /// Vector 31: Reserved.
    _reserved_31: IdtEntry,
    /// Vectors 32-255: User-defined interrupt vectors.
    pub interrupts: [IdtEntry; 224],
}

impl InterruptDescriptorTable {
    /// Creates a new IDT with all entries set to not-present.
    pub const fn new() -> Self {
        Self {
            divide_error: IdtEntry::missing(),
            debug: IdtEntry::missing(),
            nmi: IdtEntry::missing(),
            breakpoint: IdtEntry::missing(),
            overflow: IdtEntry::missing(),
            bound_range: IdtEntry::missing(),
            invalid_opcode: IdtEntry::missing(),
            device_not_available: IdtEntry::missing(),
            double_fault: IdtEntry::missing(),
            _reserved_9: IdtEntry::missing(),
            invalid_tss: IdtEntry::missing(),
            segment_not_present: IdtEntry::missing(),
            stack_segment_fault: IdtEntry::missing(),
            general_protection: IdtEntry::missing(),
            page_fault: IdtEntry::missing(),
            _reserved_15: IdtEntry::missing(),
            x87_floating_point: IdtEntry::missing(),
            alignment_check: IdtEntry::missing(),
            machine_check: IdtEntry::missing(),
            simd_floating_point: IdtEntry::missing(),
            virtualization: IdtEntry::missing(),
            control_protection: IdtEntry::missing(),
            _reserved_22_27: [IdtEntry::missing(); 6],
            hypervisor_injection: IdtEntry::missing(),
            vmm_communication: IdtEntry::missing(),
            security_exception: IdtEntry::missing(),
            _reserved_31: IdtEntry::missing(),
            interrupts: [IdtEntry::missing(); 224],
        }
    }

    /// Loads this IDT into the CPU via the `lidt` instruction.
    ///
    /// # Safety
    ///
    /// - The IDT must be `'static` (must not be dropped while loaded).
    /// - The handler functions referenced by entries must remain valid.
    pub unsafe fn load(&'static self) {
        let ptr = DescriptorTablePointer {
            limit: (core::mem::size_of::<Self>() - 1) as u16,
            base: self as *const _ as u64,
        };
        unsafe {
            core::arch::asm!(
                "lidt [{}]",
                in(reg) &ptr,
                options(readonly, nostack, preserves_flags),
            );
        }
    }
}

/// Index into interrupt vectors 32-255 using `idt[vector]`.
impl Index<u8> for InterruptDescriptorTable {
    type Output = IdtEntry;

    fn index(&self, index: u8) -> &Self::Output {
        assert!(index >= 32, "use named fields for exception vectors 0-31");
        &self.interrupts[(index - 32) as usize]
    }
}

/// Mutable index into interrupt vectors 32-255 using `idt[vector]`.
impl IndexMut<u8> for InterruptDescriptorTable {
    fn index_mut(&mut self, index: u8) -> &mut Self::Output {
        assert!(index >= 32, "use named fields for exception vectors 0-31");
        &mut self.interrupts[(index - 32) as usize]
    }
}
