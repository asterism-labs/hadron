//! Hardware interrupt dispatch subsystem.
//!
//! Provides a static handler table for vectors 32-255 and macro-generated
//! `extern "x86-interrupt"` stub functions that call a common dispatcher.
//! The dispatcher invokes the registered handler (if any) and then sends
//! LAPIC EOI.

use core::sync::atomic::{AtomicPtr, Ordering};

use hadron_core::arch::x86_64::structures::idt::InterruptStackFrame;

/// Number of hardware interrupt vectors (32-255).
const NUM_VECTORS: usize = 224;

/// Handler function signature: receives the vector number.
pub type InterruptHandler = fn(u8);

/// Static dispatch table: one atomic function pointer per vector (32-255).
/// Null means no handler registered.
static HANDLERS: [AtomicPtr<()>; NUM_VECTORS] = {
    // SAFETY: We're initializing an array of null AtomicPtrs.
    const INIT: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
    [INIT; NUM_VECTORS]
};

/// Error type for interrupt registration.
#[derive(Debug)]
pub enum InterruptError {
    /// Vector is outside the valid range (32-255).
    InvalidVector,
    /// A handler is already registered for this vector.
    AlreadyRegistered,
    /// No free vectors in the dynamic allocation range.
    VectorExhausted,
}

impl core::fmt::Display for InterruptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidVector => write!(f, "vector outside valid range 32-255"),
            Self::AlreadyRegistered => write!(f, "handler already registered for this vector"),
            Self::VectorExhausted => write!(f, "no free vectors in dynamic range"),
        }
    }
}

/// Registers a handler for the given interrupt vector (32-255).
///
/// Returns an error if the vector is out of range or already has a handler.
pub fn register_handler(vector: u8, handler: InterruptHandler) -> Result<(), InterruptError> {
    if vector < 32 {
        return Err(InterruptError::InvalidVector);
    }
    let idx = (vector - 32) as usize;
    if idx >= NUM_VECTORS {
        return Err(InterruptError::InvalidVector);
    }

    let ptr = handler as *mut ();
    let old = HANDLERS[idx].compare_exchange(
        core::ptr::null_mut(),
        ptr,
        Ordering::AcqRel,
        Ordering::Acquire,
    );

    match old {
        Ok(_) => Ok(()),
        Err(_) => Err(InterruptError::AlreadyRegistered),
    }
}

/// Unregisters the handler for the given interrupt vector.
pub fn unregister_handler(vector: u8) {
    if vector >= 32 {
        let idx = (vector - 32) as usize;
        if idx < NUM_VECTORS {
            HANDLERS[idx].store(core::ptr::null_mut(), Ordering::Release);
        }
    }
}

/// Common dispatch function called by all hardware interrupt stubs.
///
/// Looks up the handler in the dispatch table, calls it if present,
/// then sends LAPIC EOI.
fn dispatch_interrupt(vector: u8) {
    let idx = (vector - 32) as usize;
    if idx < NUM_VECTORS {
        let handler = HANDLERS[idx].load(Ordering::Acquire);
        if !handler.is_null() {
            // SAFETY: The handler was registered via `register_handler` which
            // takes a valid `fn(u8)` pointer.
            let f: InterruptHandler = unsafe { core::mem::transmute(handler) };
            f(vector);
        }
    }

    // Send LAPIC EOI. We access the LAPIC via the global reference set
    // during boot. If the LAPIC hasn't been initialized yet, skip EOI
    // (shouldn't happen in practice since interrupts are only enabled
    // after LAPIC init).
    crate::arch::x86_64::acpi::send_lapic_eoi();
}

// We generate stubs with a function array indexed by vector offset (0-223 → vectors 32-255).

/// Stub handler type matching the IDT entry signature.
type StubFn = extern "x86-interrupt" fn(InterruptStackFrame);

/// Generate a stub for a specific vector offset (0-223, maps to vectors 32-255).
macro_rules! make_stub {
    ($offset:expr) => {{
        extern "x86-interrupt" fn stub(_frame: InterruptStackFrame) {
            dispatch_interrupt($offset + 32);
        }
        stub as StubFn
    }};
}

/// Table of all 224 stub functions, one per hardware interrupt vector.
/// `STUBS[i]` handles vector `i + 32`.
// Due to the lack of const generics over function addresses, we enumerate them
// explicitly. Groups of 16 for readability.
#[allow(clippy::declare_interior_mutable_const)]
pub static STUBS: [StubFn; NUM_VECTORS] = [
    // Vectors 32-47 (ISA IRQs)
    make_stub!(0),
    make_stub!(1),
    make_stub!(2),
    make_stub!(3),
    make_stub!(4),
    make_stub!(5),
    make_stub!(6),
    make_stub!(7),
    make_stub!(8),
    make_stub!(9),
    make_stub!(10),
    make_stub!(11),
    make_stub!(12),
    make_stub!(13),
    make_stub!(14),
    make_stub!(15),
    // Vectors 48-63
    make_stub!(16),
    make_stub!(17),
    make_stub!(18),
    make_stub!(19),
    make_stub!(20),
    make_stub!(21),
    make_stub!(22),
    make_stub!(23),
    make_stub!(24),
    make_stub!(25),
    make_stub!(26),
    make_stub!(27),
    make_stub!(28),
    make_stub!(29),
    make_stub!(30),
    make_stub!(31),
    // Vectors 64-79
    make_stub!(32),
    make_stub!(33),
    make_stub!(34),
    make_stub!(35),
    make_stub!(36),
    make_stub!(37),
    make_stub!(38),
    make_stub!(39),
    make_stub!(40),
    make_stub!(41),
    make_stub!(42),
    make_stub!(43),
    make_stub!(44),
    make_stub!(45),
    make_stub!(46),
    make_stub!(47),
    // Vectors 80-95
    make_stub!(48),
    make_stub!(49),
    make_stub!(50),
    make_stub!(51),
    make_stub!(52),
    make_stub!(53),
    make_stub!(54),
    make_stub!(55),
    make_stub!(56),
    make_stub!(57),
    make_stub!(58),
    make_stub!(59),
    make_stub!(60),
    make_stub!(61),
    make_stub!(62),
    make_stub!(63),
    // Vectors 96-111
    make_stub!(64),
    make_stub!(65),
    make_stub!(66),
    make_stub!(67),
    make_stub!(68),
    make_stub!(69),
    make_stub!(70),
    make_stub!(71),
    make_stub!(72),
    make_stub!(73),
    make_stub!(74),
    make_stub!(75),
    make_stub!(76),
    make_stub!(77),
    make_stub!(78),
    make_stub!(79),
    // Vectors 112-127
    make_stub!(80),
    make_stub!(81),
    make_stub!(82),
    make_stub!(83),
    make_stub!(84),
    make_stub!(85),
    make_stub!(86),
    make_stub!(87),
    make_stub!(88),
    make_stub!(89),
    make_stub!(90),
    make_stub!(91),
    make_stub!(92),
    make_stub!(93),
    make_stub!(94),
    make_stub!(95),
    // Vectors 128-143
    make_stub!(96),
    make_stub!(97),
    make_stub!(98),
    make_stub!(99),
    make_stub!(100),
    make_stub!(101),
    make_stub!(102),
    make_stub!(103),
    make_stub!(104),
    make_stub!(105),
    make_stub!(106),
    make_stub!(107),
    make_stub!(108),
    make_stub!(109),
    make_stub!(110),
    make_stub!(111),
    // Vectors 144-159
    make_stub!(112),
    make_stub!(113),
    make_stub!(114),
    make_stub!(115),
    make_stub!(116),
    make_stub!(117),
    make_stub!(118),
    make_stub!(119),
    make_stub!(120),
    make_stub!(121),
    make_stub!(122),
    make_stub!(123),
    make_stub!(124),
    make_stub!(125),
    make_stub!(126),
    make_stub!(127),
    // Vectors 160-175
    make_stub!(128),
    make_stub!(129),
    make_stub!(130),
    make_stub!(131),
    make_stub!(132),
    make_stub!(133),
    make_stub!(134),
    make_stub!(135),
    make_stub!(136),
    make_stub!(137),
    make_stub!(138),
    make_stub!(139),
    make_stub!(140),
    make_stub!(141),
    make_stub!(142),
    make_stub!(143),
    // Vectors 176-191
    make_stub!(144),
    make_stub!(145),
    make_stub!(146),
    make_stub!(147),
    make_stub!(148),
    make_stub!(149),
    make_stub!(150),
    make_stub!(151),
    make_stub!(152),
    make_stub!(153),
    make_stub!(154),
    make_stub!(155),
    make_stub!(156),
    make_stub!(157),
    make_stub!(158),
    make_stub!(159),
    // Vectors 192-207
    make_stub!(160),
    make_stub!(161),
    make_stub!(162),
    make_stub!(163),
    make_stub!(164),
    make_stub!(165),
    make_stub!(166),
    make_stub!(167),
    make_stub!(168),
    make_stub!(169),
    make_stub!(170),
    make_stub!(171),
    make_stub!(172),
    make_stub!(173),
    make_stub!(174),
    make_stub!(175),
    // Vectors 208-223
    make_stub!(176),
    make_stub!(177),
    make_stub!(178),
    make_stub!(179),
    make_stub!(180),
    make_stub!(181),
    make_stub!(182),
    make_stub!(183),
    make_stub!(184),
    make_stub!(185),
    make_stub!(186),
    make_stub!(187),
    make_stub!(188),
    make_stub!(189),
    make_stub!(190),
    make_stub!(191),
    // Vectors 224-239
    make_stub!(192),
    make_stub!(193),
    make_stub!(194),
    make_stub!(195),
    make_stub!(196),
    make_stub!(197),
    make_stub!(198),
    make_stub!(199),
    make_stub!(200),
    make_stub!(201),
    make_stub!(202),
    make_stub!(203),
    make_stub!(204),
    make_stub!(205),
    make_stub!(206),
    make_stub!(207),
    // Vectors 240-255 (IPI / timer / spurious)
    make_stub!(208),
    make_stub!(209),
    make_stub!(210),
    make_stub!(211),
    make_stub!(212),
    make_stub!(213),
    make_stub!(214),
    make_stub!(215),
    make_stub!(216),
    make_stub!(217),
    make_stub!(218),
    make_stub!(219),
    make_stub!(220),
    make_stub!(221),
    make_stub!(222),
    make_stub!(223),
];

/// Well-known vector assignments.
pub mod vectors {
    /// LAPIC timer vector.
    pub const TIMER: u8 = 254;
    /// Spurious interrupt vector.
    pub const SPURIOUS: u8 = 255;
    /// First vector available for dynamic allocation.
    pub const DYNAMIC_START: u8 = 48;
    /// Last vector available for dynamic allocation.
    pub const DYNAMIC_END: u8 = 239;
    /// First IPI vector.
    pub const IPI_START: u8 = 240;
    /// Last IPI vector.
    pub const IPI_END: u8 = 253;

    /// Returns the interrupt vector for an ISA IRQ (0-15).
    #[must_use]
    pub const fn isa_irq_vector(irq: u8) -> u8 {
        32 + irq
    }
}

/// Allocates a free vector in the dynamic range (48-239).
///
/// Performs a linear scan of the handler table for the first unregistered slot.
pub fn alloc_vector() -> Result<u8, InterruptError> {
    for vector in vectors::DYNAMIC_START..=vectors::DYNAMIC_END {
        let idx = (vector - 32) as usize;
        if HANDLERS[idx].load(Ordering::Acquire).is_null() {
            return Ok(vector);
        }
    }
    Err(InterruptError::VectorExhausted)
}
