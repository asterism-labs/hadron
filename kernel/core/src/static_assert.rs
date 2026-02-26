//! Compile-time static assertion macro.

/// Asserts a condition at compile time.
///
/// # Examples
///
/// ```ignore
/// use crate::static_assert;
/// static_assert!(core::mem::size_of::<u64>() == 8);
/// static_assert!(1 + 1 == 2, "math is broken");
/// ```
#[macro_export]
macro_rules! static_assert {
    ($cond:expr $(,)?) => {
        const _: () = assert!($cond);
    };
    ($cond:expr, $msg:expr $(,)?) => {
        const _: () = assert!($cond, $msg);
    };
}
