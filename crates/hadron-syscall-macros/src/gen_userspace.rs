//! Code generation for the `userspace` feature: raw `syscallN` asm stubs and typed wrappers.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::model::SyscallDefs;

/// Generate userspace-only items gated behind `#[cfg(feature = "userspace")]`.
pub(crate) fn generate(defs: &SyscallDefs) -> TokenStream {
    let raw_stubs = gen_raw_stubs();
    let wrappers = gen_typed_wrappers(defs);

    quote! {
        #[cfg(feature = "userspace")]
        #raw_stubs

        #[cfg(feature = "userspace")]
        #wrappers
    }
}

/// Generate raw `syscall0` through `syscall5` inline asm functions.
fn gen_raw_stubs() -> TokenStream {
    quote! {
        /// Raw syscall assembly wrappers using inline assembly.
        ///
        /// Hadron syscall ABI: `RAX` = syscall number, arguments in
        /// `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9`. Return value in `RAX`.
        /// `RCX` and `R11` are clobbered by the `syscall` instruction.
        /// All other caller-saved registers (`RDI`, `RSI`, `RDX`, `R8`,
        /// `R9`, `R10`) may be clobbered by the kernel and must be declared.
        pub mod raw {
            /// Issue a syscall with 0 arguments.
            #[inline(always)]
            pub fn syscall0(nr: usize) -> isize {
                let ret: isize;
                // SAFETY: Invokes the kernel syscall handler with the given number.
                // The syscall instruction is the defined userspace-to-kernel transition.
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        inlateout("rax") nr as isize => ret,
                        lateout("rcx") _,
                        lateout("r11") _,
                        lateout("rdi") _,
                        lateout("rsi") _,
                        lateout("rdx") _,
                        lateout("r8") _,
                        lateout("r9") _,
                        lateout("r10") _,
                        options(nostack),
                    );
                }
                ret
            }

            /// Issue a syscall with 1 argument.
            #[inline(always)]
            pub fn syscall1(nr: usize, a0: usize) -> isize {
                let ret: isize;
                // SAFETY: Same as syscall0, with one argument in RDI.
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        inlateout("rax") nr as isize => ret,
                        in("rdi") a0,
                        lateout("rcx") _,
                        lateout("r11") _,
                        lateout("rsi") _,
                        lateout("rdx") _,
                        lateout("r8") _,
                        lateout("r9") _,
                        lateout("r10") _,
                        options(nostack),
                    );
                }
                ret
            }

            /// Issue a syscall with 2 arguments.
            #[inline(always)]
            pub fn syscall2(nr: usize, a0: usize, a1: usize) -> isize {
                let ret: isize;
                // SAFETY: Same as syscall0, with two arguments in RDI, RSI.
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        inlateout("rax") nr as isize => ret,
                        in("rdi") a0,
                        in("rsi") a1,
                        lateout("rcx") _,
                        lateout("r11") _,
                        lateout("rdx") _,
                        lateout("r8") _,
                        lateout("r9") _,
                        lateout("r10") _,
                        options(nostack),
                    );
                }
                ret
            }

            /// Issue a syscall with 3 arguments.
            #[inline(always)]
            pub fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> isize {
                let ret: isize;
                // SAFETY: Same as syscall0, with three arguments in RDI, RSI, RDX.
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        inlateout("rax") nr as isize => ret,
                        in("rdi") a0,
                        in("rsi") a1,
                        in("rdx") a2,
                        lateout("rcx") _,
                        lateout("r11") _,
                        lateout("r8") _,
                        lateout("r9") _,
                        lateout("r10") _,
                        options(nostack),
                    );
                }
                ret
            }

            /// Issue a syscall with 4 arguments.
            #[inline(always)]
            pub fn syscall4(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
                let ret: isize;
                // SAFETY: Same as syscall0, with four arguments in RDI, RSI, RDX, R10.
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        inlateout("rax") nr as isize => ret,
                        in("rdi") a0,
                        in("rsi") a1,
                        in("rdx") a2,
                        in("r10") a3,
                        lateout("rcx") _,
                        lateout("r11") _,
                        lateout("r8") _,
                        lateout("r9") _,
                        options(nostack),
                    );
                }
                ret
            }

            /// Issue a syscall with 5 arguments.
            #[inline(always)]
            pub fn syscall5(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
                let ret: isize;
                // SAFETY: Same as syscall0, with five arguments in RDI, RSI, RDX, R10, R8.
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        inlateout("rax") nr as isize => ret,
                        in("rdi") a0,
                        in("rsi") a1,
                        in("rdx") a2,
                        in("r10") a3,
                        in("r8") a4,
                        lateout("rcx") _,
                        lateout("r11") _,
                        lateout("r9") _,
                        options(nostack),
                    );
                }
                ret
            }
        }
    }
}

/// Generate typed wrapper functions for non-reserved syscalls.
fn gen_typed_wrappers(defs: &SyscallDefs) -> TokenStream {
    let mut wrappers = Vec::new();

    for group in &defs.groups {
        for syscall in &group.syscalls {
            // Skip reserved syscalls.
            if syscall.reserved.is_some() {
                continue;
            }

            let fn_name = format_ident!("sys_{}", syscall.name);
            let const_name = format_ident!("SYS_{}", syscall.name.to_string().to_uppercase());
            let attrs = &syscall.attrs;
            let arg_count = syscall.args.len();

            let params: Vec<_> = syscall
                .args
                .iter()
                .map(|a| {
                    let name = &a.name;
                    quote! { #name: usize }
                })
                .collect();

            let arg_names: Vec<_> = syscall.args.iter().map(|a| &a.name).collect();

            let syscall_fn = format_ident!("syscall{}", arg_count);

            wrappers.push(quote! {
                #(#attrs)*
                #[inline]
                pub fn #fn_name(#(#params),*) -> isize {
                    raw::#syscall_fn(#const_name, #(#arg_names),*)
                }
            });
        }
    }

    quote! {
        /// Typed syscall wrappers for non-reserved syscalls.
        pub mod wrappers {
            use super::*;
            #(#wrappers)*
        }
    }
}
