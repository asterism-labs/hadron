//! Safe wrappers for linker-section-based data.
//!
//! This crate encapsulates the unsafe patterns required to read typed data from
//! linker sections behind safe declarative macros. It provides three macros:
//!
//! - [`declare_linkset!`] — declares a function that returns a typed `&'static [T]`
//!   from a linker section bounded by `__<section>_start` / `__<section>_end` symbols.
//! - [`linkset_entry!`] — places a typed static into the matching linker section.
//! - [`declare_linkset_blob!`] — declares a function that returns a raw `&'static [u8]`
//!   from a linker section (for binary blobs like HKIF).

#![no_std]
#![warn(missing_docs)]

/// Declares a function that returns a typed slice from a linker section.
///
/// The linker script must define `__<section>_start` and `__<section>_end`
/// symbols bounding the section.
///
/// # Examples
///
/// ```ignore
/// hadron_linkset::declare_linkset! {
///     /// Returns all PCI driver entries.
///     pub fn pci_driver_entries() -> [PciDriverEntry],
///     section = "hadron_pci_drivers"
/// }
/// ```
#[macro_export]
macro_rules! declare_linkset {
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident() -> [$ty:ty],
        section = $section:literal
    ) => {
        $(#[$meta])*
        $vis fn $name() -> &'static [$ty] {
            unsafe extern "C" {
                #[link_name = concat!("__", $section, "_start")]
                static LINKSET_START: u8;
                #[link_name = concat!("__", $section, "_end")]
                static LINKSET_END: u8;
            }

            // SAFETY: The linker script defines these symbols at the boundaries
            // of the named section. The section contains only `T` values placed
            // by `linkset_entry!` or `#[hadron_driver]`. The symbols remain valid
            // for the lifetime of the kernel image.
            unsafe {
                let start = ::core::ptr::addr_of!(LINKSET_START).cast::<$ty>();
                let end = ::core::ptr::addr_of!(LINKSET_END).cast::<$ty>();
                let count = end.offset_from(start) as usize;
                if count == 0 {
                    return &[];
                }
                ::core::slice::from_raw_parts(start, count)
            }
        }
    };
}

/// Places a typed static into the named linker section.
///
/// # Examples
///
/// ```ignore
/// hadron_linkset::linkset_entry!("hadron_pci_drivers",
///     AHCI_ENTRY: PciDriverEntry = PciDriverEntry { ... }
/// );
/// ```
#[macro_export]
macro_rules! linkset_entry {
    ($section:literal, $name:ident : $ty:ty = $expr:expr) => {
        #[used]
        #[unsafe(link_section = concat!(".", $section))]
        static $name: $ty = $expr;
    };
}

/// Declares a function that returns a raw byte slice from a linker section.
///
/// Use this for binary blobs (e.g., HKIF data) where the section contains
/// untyped bytes rather than an array of structs.
///
/// # Examples
///
/// ```ignore
/// hadron_linkset::declare_linkset_blob! {
///     /// Returns the embedded HKIF data.
///     pub fn hkif_data() -> &[u8],
///     section = "hadron_hkif"
/// }
/// ```
#[macro_export]
macro_rules! declare_linkset_blob {
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident() -> &[u8],
        section = $section:literal
    ) => {
        $(#[$meta])*
        $vis fn $name() -> &'static [u8] {
            unsafe extern "C" {
                #[link_name = concat!("__", $section, "_start")]
                static LINKSET_START: u8;
                #[link_name = concat!("__", $section, "_end")]
                static LINKSET_END: u8;
            }

            // SAFETY: The linker script defines these symbols at the boundaries
            // of the named section. The region is contiguous, immutable, and
            // remains valid for the lifetime of the kernel image.
            unsafe {
                let start = ::core::ptr::addr_of!(LINKSET_START);
                let end = ::core::ptr::addr_of!(LINKSET_END);
                let size = end.offset_from(start) as usize;
                if size == 0 {
                    return &[];
                }
                ::core::slice::from_raw_parts(start, size)
            }
        }
    };
}
