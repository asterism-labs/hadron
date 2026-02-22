//! Hardware interrupt dispatch subsystem.
//!
//! Provides a static handler table for vectors 32-255 and macro-generated
//! naked stub functions that call a common dispatcher. Each stub checks the
//! interrupted privilege level (RPL in the saved CS) and conditionally
//! performs `swapgs` so that per-CPU state is accessible regardless of
//! whether the interrupt fired from ring 0 or ring 3.
//!
//! The dispatcher invokes the registered handler (if any) and then sends
//! LAPIC EOI.

use core::sync::atomic::{AtomicPtr, Ordering};

use crate::id::{HwIrqVector, IrqVector};

/// Number of hardware interrupt vectors (32-255).
const NUM_VECTORS: usize = 224;

/// Handler function signature: receives the vector number.
pub type InterruptHandler = fn(IrqVector);

// ---------------------------------------------------------------------------
// HandlerSlot — encapsulated transmute for function pointers
// ---------------------------------------------------------------------------

/// A single slot in the hardware interrupt dispatch table.
///
/// Wraps an [`AtomicPtr<()>`] that stores either null (no handler) or a valid
/// [`InterruptHandler`] function pointer cast to `*mut ()`. The transmute
/// between `*mut ()` and `fn(IrqVector)` is encapsulated here with a
/// documented invariant: only [`try_set`](HandlerSlot::try_set) can store
/// non-null values, and it only accepts valid `InterruptHandler` pointers.
#[repr(transparent)]
struct HandlerSlot(AtomicPtr<()>);

impl HandlerSlot {
    /// An empty (no handler) slot.
    const EMPTY: Self = Self(AtomicPtr::new(core::ptr::null_mut()));

    /// Atomically sets the handler if the slot is currently empty.
    ///
    /// Returns `Ok(())` on success, `Err(())` if a handler is already registered.
    fn try_set(&self, handler: InterruptHandler) -> Result<(), ()> {
        self.0
            .compare_exchange(
                core::ptr::null_mut(),
                handler as *mut (),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|_| ())
    }

    /// Clears the handler slot, setting it back to empty.
    fn clear(&self) {
        self.0.store(core::ptr::null_mut(), Ordering::Release);
    }

    /// Dispatches to the registered handler (if any).
    ///
    /// # Safety invariant
    ///
    /// The transmute is sound because [`try_set`](HandlerSlot::try_set) only
    /// stores valid `InterruptHandler` function pointers, and
    /// [`clear`](HandlerSlot::clear) only stores null.
    fn dispatch(&self, vector: IrqVector) {
        let ptr = self.0.load(Ordering::Acquire);
        if !ptr.is_null() {
            // SAFETY: `try_set` guarantees all non-null values are valid
            // `InterruptHandler` pointers.
            let f: InterruptHandler = unsafe { core::mem::transmute(ptr) };
            f(vector);
        }
    }

    /// Returns `true` if no handler is currently registered.
    fn is_empty(&self) -> bool {
        self.0.load(Ordering::Acquire).is_null()
    }
}

/// Static dispatch table: one handler slot per vector (32-255).
static HANDLERS: [HandlerSlot; NUM_VECTORS] = {
    const INIT: HandlerSlot = HandlerSlot::EMPTY;
    [INIT; NUM_VECTORS]
};

/// Error type for interrupt registration.
#[derive(Debug)]
pub enum InterruptError {
    /// A handler is already registered for this vector.
    AlreadyRegistered,
    /// No free vectors in the dynamic allocation range.
    VectorExhausted,
}

impl core::fmt::Display for InterruptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AlreadyRegistered => write!(f, "handler already registered for this vector"),
            Self::VectorExhausted => write!(f, "no free vectors in dynamic range"),
        }
    }
}

/// Registers a handler for the given hardware interrupt vector.
///
/// Returns an error if the vector already has a handler.
pub fn register_handler(
    vector: HwIrqVector,
    handler: InterruptHandler,
) -> Result<(), InterruptError> {
    debug_assert!(
        handler as *const () != core::ptr::null(),
        "register_handler: null handler function pointer"
    );
    match HANDLERS[vector.table_index()].try_set(handler) {
        Ok(()) => {
            crate::ktrace_subsys!(irq, "registered handler for vector {}", vector);
            Ok(())
        }
        Err(()) => Err(InterruptError::AlreadyRegistered),
    }
}

/// Unregisters the handler for the given hardware interrupt vector.
pub fn unregister_handler(vector: HwIrqVector) {
    HANDLERS[vector.table_index()].clear();
    crate::ktrace_subsys!(irq, "unregistered handler for vector {}", vector);
}

/// Common dispatch function called by all hardware interrupt stubs.
///
/// Looks up the handler in the dispatch table, calls it if present,
/// then sends LAPIC EOI.
///
/// # ABI
///
/// Uses the C calling convention so naked stubs can call it directly
/// via `call` with the vector number in `edi`.
extern "C" fn dispatch_interrupt(vector: u8) {
    // Debug: verify GS_BASE points to valid per-CPU data. If an interrupt
    // stub failed to swapgs, GS_BASE is the user value (typically 0) and
    // this read will fault or return a non-kernel pointer.
    debug_assert!(
        hadron_core::cpu_local::cpu_is_initialized(),
        "dispatch_interrupt: GS_BASE does not point to valid per-CPU data \
         (vector {vector}). Likely a missing swapgs in the interrupt stub."
    );

    let idx = (vector - 32) as usize;
    if idx < NUM_VECTORS {
        HANDLERS[idx].dispatch(IrqVector::new(vector));
    }

    // Send LAPIC EOI. We access the LAPIC via the global reference set
    // during boot. If the LAPIC hasn't been initialized yet, skip EOI
    // (shouldn't happen in practice since interrupts are only enabled
    // after LAPIC init).
    crate::arch::x86_64::acpi::Acpi::send_lapic_eoi();
}

// We generate stubs with a function array indexed by vector offset (0-223 → vectors 32-255).
// Each stub is a naked function that:
//   1. Checks the RPL bits in the interrupted CS to determine ring 0 vs ring 3
//   2. Conditionally swapgs on entry (ring 3 → kernel GS)
//   3. Saves/restores scratch registers around the call to dispatch_interrupt
//   4. Conditionally swapgs on exit (ring 3 → user GS)
//   5. Returns via iretq

/// Stub handler type: raw function address for IDT entries.
pub type StubFn = unsafe extern "C" fn();

/// Generate a naked interrupt stub for a specific vector offset (0-223, maps to vectors 32-255).
///
/// The stub checks the RPL in the interrupted CS and conditionally performs
/// `swapgs` so that kernel GS_BASE is always active when `dispatch_interrupt`
/// runs. This is critical for interrupts that fire from ring 3 (userspace).
macro_rules! make_stub {
    ($offset:expr) => {{
        #[unsafe(naked)]
        unsafe extern "C" fn stub() {
            core::arch::naked_asm!(
                // ── Check privilege level of interrupted code ──
                // On x86_64, the CPU always pushes SS, RSP, RFLAGS, CS, RIP
                // (5 qwords). CS is at [rsp + 8]; RPL bits [0:1] indicate ring.
                "test qword ptr [rsp + 8], 3",
                "jz 1f",
                "swapgs",                       // Ring 3 → swap to kernel GS
                "1:",

                // ── Save scratch registers ──
                // 9 pushes = 72 bytes. With the 40-byte interrupt frame, the
                // total displacement from pre-interrupt RSP is 112 = 16*7,
                // keeping RSP 16-byte aligned for the call.
                "push rax",
                "push rcx",
                "push rdx",
                "push rsi",
                "push rdi",
                "push r8",
                "push r9",
                "push r10",
                "push r11",

                // ── Dispatch ──
                // Clear DF for the C ABI (the interrupted code may have set
                // it via STD). The original RFLAGS — including DF — is
                // restored by iretq, so the interrupted code is unaffected.
                "cld",
                "mov edi, {vector}",            // arg0: vector number
                "call {dispatch}",

                // ── Restore scratch registers ──
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop rdi",
                "pop rsi",
                "pop rdx",
                "pop rcx",
                "pop rax",

                // ── Check privilege level again before return ──
                "test qword ptr [rsp + 8], 3",
                "jz 2f",
                "swapgs",                       // Returning to ring 3 → swap back
                "2:",
                "iretq",

                vector   = const ($offset + 32u8),
                dispatch = sym dispatch_interrupt,
            );
        }
        stub as StubFn
    }};
}

/// Generates the complete stub table as a single array literal.
///
/// Rust macro invocations in expression position produce a single expression,
/// so a `make_stub_group!` that expands to `a, b, c` cannot splice commas into
/// an outer `[...]`. Instead, this macro takes the full list of offsets and
/// produces the complete array in one expansion.
macro_rules! make_stub_table {
    ($($offset:expr),* $(,)?) => {
        [$(make_stub!($offset)),*]
    };
}

/// Table of all 224 stub functions, one per hardware interrupt vector.
/// `STUBS[i]` handles vector `i + 32`.
pub static STUBS: [StubFn; NUM_VECTORS] = make_stub_table![
    // Vectors  32- 47 (ISA IRQs)
      0,   1,   2,   3,   4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,
    // Vectors  48- 63
     16,  17,  18,  19,  20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,
    // Vectors  64- 79
     32,  33,  34,  35,  36,  37,  38,  39,  40,  41,  42,  43,  44,  45,  46,  47,
    // Vectors  80- 95
     48,  49,  50,  51,  52,  53,  54,  55,  56,  57,  58,  59,  60,  61,  62,  63,
    // Vectors  96-111
     64,  65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  76,  77,  78,  79,
    // Vectors 112-127
     80,  81,  82,  83,  84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,
    // Vectors 128-143
     96,  97,  98,  99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111,
    // Vectors 144-159
    112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127,
    // Vectors 160-175
    128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143,
    // Vectors 176-191
    144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159,
    // Vectors 192-207
    160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175,
    // Vectors 208-223
    176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191,
    // Vectors 224-239
    192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207,
    // Vectors 240-255 (IPI / timer / spurious)
    208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223,
];

hadron_core::static_assert!(STUBS.len() == NUM_VECTORS);

/// Well-known vector assignments.
pub mod vectors {
    use crate::id::HwIrqVector;

    /// LAPIC timer vector.
    pub const TIMER: HwIrqVector = HwIrqVector::new(254);
    /// Spurious interrupt vector.
    pub const SPURIOUS: HwIrqVector = HwIrqVector::new(255);
    /// First vector available for dynamic allocation.
    pub const DYNAMIC_START: u8 = 48;
    /// Last vector available for dynamic allocation.
    pub const DYNAMIC_END: u8 = 239;
    /// First IPI vector.
    pub const IPI_START: HwIrqVector = HwIrqVector::new(240);
    /// Last IPI vector.
    pub const IPI_END: HwIrqVector = HwIrqVector::new(253);

    /// Returns the interrupt vector for an ISA IRQ (0-15).
    #[must_use]
    pub const fn isa_irq_vector(irq: u8) -> HwIrqVector {
        HwIrqVector::new(32 + irq)
    }
}

/// Allocates a free vector in the dynamic range (48-239).
///
/// Performs a linear scan of the handler table for the first unregistered slot.
pub fn alloc_vector() -> Result<HwIrqVector, InterruptError> {
    for raw in vectors::DYNAMIC_START..=vectors::DYNAMIC_END {
        let idx = (raw - 32) as usize;
        if HANDLERS[idx].is_empty() {
            return Ok(HwIrqVector::new(raw));
        }
    }
    Err(InterruptError::VectorExhausted)
}
