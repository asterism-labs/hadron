//! Device filesystem — kernel glue.
//!
//! Re-exports DevFs, DevNull, DevZero from `hadron-fs`.
//! Adds the kernel-specific DevConsole device that delegates to the active TTY.

pub use hadron_fs::devfs::*;

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use super::{DirEntry, FsError, Inode, InodeType, Permissions};

// ── /dev/console ───────────────────────────────────────────────────────

/// `/dev/console` — delegates to the currently active TTY.
///
/// Reads are forwarded to the active virtual terminal's line discipline.
/// Writes go to kernel console output. This provides backward compatibility
/// for code that opens `/dev/console` directly.
pub struct DevConsole;

impl Inode for DevConsole {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn dev_number(&self) -> hadron_fs::DevNumber {
        hadron_fs::DevNumber::CONSOLE
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        // Delegate to the active TTY's read path.
        crate::tty::device::tty_read_future(crate::tty::active_tty(), buf)
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if let Ok(s) = core::str::from_utf8(buf) {
                crate::kprint!("{}", s);
            } else {
                for &byte in buf {
                    crate::kprint!("{}", byte as char);
                }
            }
            Ok(buf.len())
        })
    }

    fn lookup<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn ioctl(&self, cmd: u32, arg: usize) -> Result<usize, FsError> {
        use crate::syscall::userptr::UserPtr;
        use hadron_syscall::{
            TCGETS, TCSETS, TCSETSF, TCSETSW, TIOCGPGRP, TIOCGWINSZ, TIOCSPGRP, TIOCSWINSZ,
            Termios, Winsize,
        };

        let tty = crate::tty::active_tty();
        match cmd {
            TCGETS => {
                let t = tty.get_termios();
                let ptr = UserPtr::<Termios>::new(arg).map_err(|_| FsError::Fault)?;
                // SAFETY: UserPtr validated address is user-space and aligned.
                unsafe { ptr.write(t) };
                Ok(0)
            }
            TCSETS | TCSETSW | TCSETSF => {
                let ptr = UserPtr::<Termios>::new(arg).map_err(|_| FsError::Fault)?;
                // SAFETY: UserPtr validated address; reading a POD struct.
                let t = unsafe { *ptr.as_ref() };
                tty.set_termios(&t);
                Ok(0)
            }
            TIOCGPGRP => {
                let pgid = tty.foreground_pgid().unwrap_or(0);
                let ptr = UserPtr::<u32>::new(arg).map_err(|_| FsError::Fault)?;
                // SAFETY: UserPtr validated address is user-space and aligned.
                unsafe { ptr.write(pgid) };
                Ok(0)
            }
            TIOCSPGRP => {
                let ptr = UserPtr::<u32>::new(arg).map_err(|_| FsError::Fault)?;
                // SAFETY: UserPtr validated address; reading a u32.
                let pgid = unsafe { *ptr.as_ref() };
                tty.set_foreground_pgid(pgid);
                Ok(0)
            }
            TIOCGWINSZ => {
                let ws = tty.get_winsize();
                let ptr = UserPtr::<Winsize>::new(arg).map_err(|_| FsError::Fault)?;
                // SAFETY: UserPtr validated address is user-space and aligned.
                unsafe { ptr.write(ws) };
                Ok(0)
            }
            TIOCSWINSZ => Ok(0),
            _ => Err(FsError::NotSupported),
        }
    }

    fn poll_readiness(&self, waker: Option<&core::task::Waker>) -> u16 {
        use hadron_syscall::{POLLIN, POLLOUT};

        let tty = crate::tty::active_tty();
        if let Some(w) = waker {
            tty.subscribe(w);
        }
        tty.poll_hardware();
        let mut events = POLLOUT;
        if tty.has_input() {
            events |= POLLIN;
        }
        events
    }
}
