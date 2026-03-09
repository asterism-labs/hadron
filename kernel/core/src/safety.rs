//! Runtime safety checks for unsafe preconditions.
//!
//! The [`assert_unsafe_precondition!`] macro guards conditions that must hold
//! for subsequent unsafe code to be sound. Unlike `debug_assert!` (which
//! checks logic invariants), this macro specifically documents and enforces
//! safety invariants whose violation would be undefined behavior.
//!
//! # Behavior
//!
//! | Build configuration | Result |
//! |---------------------|--------|
//! | Debug (`debug_assertions`) | Panics on failure |
//! | Release + `hadron_hardened` cfg | Panics on failure |
//! | Release (default) | Compiled away (condition still type-checked) |

/// Checks a precondition that must hold for subsequent `unsafe` code to be
/// sound.
///
/// - **Debug builds**: panics on failure with the stringified condition.
/// - **Release + `hadron_hardened` cfg**: panics on failure.
/// - **Release** (default): compiles away, but the condition expression is
///   still type-checked to prevent bitrot.
///
/// # When to use
///
/// Use `assert_unsafe_precondition!` to guard invariants whose violation would
/// cause *undefined behavior* (e.g., alignment requirements for page table
/// operations, non-null pointers before dereference).
///
/// Use `debug_assert!` for logic invariants whose violation is a bug but not
/// UB.
///
/// # Examples
///
/// ```ignore
/// assert_unsafe_precondition!(phys_addr.is_aligned(4096));
/// assert_unsafe_precondition!(size > 0, "MMIO region size must be non-zero");
/// ```
#[macro_export]
macro_rules! assert_unsafe_precondition {
    ($cond:expr $(,)?) => {
        #[cfg(any(debug_assertions, hadron_hardened))]
        {
            if !$cond {
                panic!(
                    "unsafe precondition violated: {}",
                    stringify!($cond),
                );
            }
        }
        #[cfg(not(any(debug_assertions, hadron_hardened)))]
        {
            if false {
                let _ = $cond;
            }
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        #[cfg(any(debug_assertions, hadron_hardened))]
        {
            if !$cond {
                panic!($($arg)+);
            }
        }
        #[cfg(not(any(debug_assertions, hadron_hardened)))]
        {
            if false {
                let _ = $cond;
            }
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn passing_precondition() {
        assert_unsafe_precondition!(1 + 1 == 2);
    }

    #[test]
    fn passing_precondition_with_message() {
        assert_unsafe_precondition!(true, "should not panic");
    }

    #[test]
    #[should_panic(expected = "unsafe precondition violated")]
    fn failing_precondition() {
        assert_unsafe_precondition!(false);
    }

    #[test]
    #[should_panic(expected = "custom message")]
    fn failing_precondition_with_message() {
        assert_unsafe_precondition!(false, "custom message");
    }

    #[test]
    fn format_args_in_message() {
        let val = 42;
        assert_unsafe_precondition!(val > 0, "value must be positive, got {}", val);
    }
}
