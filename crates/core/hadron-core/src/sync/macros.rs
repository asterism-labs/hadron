//! Internal macros for the sync module.

/// Emit a function that is `const` on non-loom builds but plain `fn` under
/// loom (whose constructors are not const-evaluable).
///
/// # Example
///
/// ```ignore
/// maybe_const_fn! {
///     pub fn new(value: u32) -> Self {
///         Self { value }
///     }
/// }
/// ```
macro_rules! maybe_const_fn {
    (
        $(#[$attr:meta])*
        $vis:vis fn $name:ident $(<$($gen:tt)*>)? ($($param:ident : $pty:ty),* $(,)?) -> $ret:ty
            $body:block
    ) => {
        $(#[$attr])*
        #[cfg(not(loom))]
        $vis const fn $name $(<$($gen)*>)? ($($param: $pty),*) -> $ret $body

        $(#[$attr])*
        #[cfg(loom)]
        $vis fn $name $(<$($gen)*>)? ($($param: $pty),*) -> $ret $body
    };
}
