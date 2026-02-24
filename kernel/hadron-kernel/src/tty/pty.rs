//! Pseudoterminal (PTY) support.
//!
//! A PTY pair consists of a master and slave. The slave behaves like a real
//! terminal (with line discipline and termios), while the master provides raw
//! access for terminal emulators.
//!
//! Data flow:
//! - Master write → slave input buffer → line discipline → slave read
//! - Slave write → master input buffer → master read

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::Waker;
use hadron_core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};
use crate::sync::{HeapWaitQueue, IrqSpinLock, SpinLock};
use hadron_syscall::Termios;

use super::ldisc::LineDiscipline;

/// Default PTY buffer size: 16 KiB per direction.
const PTY_BUF_SIZE: usize = 16 * 1024;

/// Maximum number of PTY pairs.
pub const MAX_PTYS: usize = 64;

/// Shared state between master and slave halves.
struct PtyInner {
    /// PTY index (0..MAX_PTYS).
    index: usize,
    /// Buffer: master→slave direction (master writes, slave reads after ldisc).
    m2s_buf: SpinLock<CircularBuffer>,
    /// Buffer: slave→master direction (slave writes, master reads).
    s2m_buf: SpinLock<CircularBuffer>,
    /// Wake master reader when slave writes data.
    master_wq: HeapWaitQueue,
    /// Wake slave reader when master writes data (via line discipline).
    slave_wq: HeapWaitQueue,
    /// Line discipline for slave side (cooked mode editing).
    ldisc: IrqSpinLock<LineDiscipline>,
    /// Terminal settings for the slave.
    termios: IrqSpinLock<Termios>,
    /// Foreground process group of the slave.
    foreground_pgid: AtomicU32,
    /// Window size.
    winsize: SpinLock<hadron_syscall::Winsize>,
    /// Whether the slave is locked (unlockpt not yet called).
    locked: AtomicBool,
    /// Number of master handles.
    masters: AtomicUsize,
    /// Number of slave handles.
    slaves: AtomicUsize,
}

/// Simple fixed-size circular buffer (same design as pipe).
struct CircularBuffer {
    data: Box<[u8]>,
    read_pos: usize,
    write_pos: usize,
    count: usize,
}

impl CircularBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: alloc::vec![0u8; capacity].into_boxed_slice(),
            read_pos: 0,
            write_pos: 0,
            count: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn is_full(&self) -> bool {
        self.count == self.data.len()
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = buf.len().min(self.count);
        let cap = self.data.len();
        for i in 0..to_read {
            buf[i] = self.data[(self.read_pos + i) % cap];
        }
        self.read_pos = (self.read_pos + to_read) % cap;
        self.count -= to_read;
        to_read
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        let available = self.data.len() - self.count;
        let to_write = buf.len().min(available);
        let cap = self.data.len();
        for i in 0..to_write {
            self.data[(self.write_pos + i) % cap] = buf[i];
        }
        self.write_pos = (self.write_pos + to_write) % cap;
        self.count += to_write;
        to_write
    }
}

/// Master side of a PTY pair.
pub struct PtyMaster(Arc<PtyInner>);

/// Slave side of a PTY pair.
pub struct PtySlave(Arc<PtyInner>);

/// Allocate a new PTY pair.
///
/// Returns `(master, slave, index)` or `None` if all PTY slots are in use.
pub fn alloc_pty() -> Option<(Arc<PtyMaster>, Arc<PtySlave>, usize)> {
    let index = PTY_ALLOC.fetch_add(1, Ordering::Relaxed);
    if index >= MAX_PTYS {
        PTY_ALLOC.fetch_sub(1, Ordering::Relaxed);
        return None;
    }

    let inner = Arc::new(PtyInner {
        index,
        m2s_buf: SpinLock::named("pty_m2s", CircularBuffer::new(PTY_BUF_SIZE)),
        s2m_buf: SpinLock::named("pty_s2m", CircularBuffer::new(PTY_BUF_SIZE)),
        master_wq: HeapWaitQueue::new(),
        slave_wq: HeapWaitQueue::new(),
        ldisc: IrqSpinLock::named("pty_ldisc", LineDiscipline::new()),
        termios: IrqSpinLock::named("pty_termios", super::default_termios()),
        foreground_pgid: AtomicU32::new(0),
        winsize: SpinLock::named(
            "pty_winsize",
            hadron_syscall::Winsize {
                rows: 25,
                cols: 80,
                xpixel: 0,
                ypixel: 0,
            },
        ),
        locked: AtomicBool::new(true),
        masters: AtomicUsize::new(1),
        slaves: AtomicUsize::new(1),
    });

    // Store slave inode in the global table for /dev/pts/N lookup.
    let slave = Arc::new(PtySlave(inner.clone()));
    {
        let mut table = PTY_SLAVES.lock();
        table[index] = Some(slave.clone() as Arc<PtySlave>);
    }

    let master = Arc::new(PtyMaster(inner));
    Some((master, slave, index))
}

/// Get a slave PTY by index (for /dev/pts/N).
pub fn get_slave(index: usize) -> Option<Arc<PtySlave>> {
    if index >= MAX_PTYS {
        return None;
    }
    let table = PTY_SLAVES.lock();
    table[index].clone()
}

/// Next PTY index to allocate.
static PTY_ALLOC: AtomicUsize = AtomicUsize::new(0);

/// Global table of slave PTY inodes for /dev/pts/ lookup.
static PTY_SLAVES: SpinLock<[Option<Arc<PtySlave>>; MAX_PTYS]> =
    SpinLock::named("pty_slaves", [const { None }; MAX_PTYS]);

// ── Master side ─────────────────────────────────────────────────────

impl Drop for PtyMaster {
    fn drop(&mut self) {
        self.0.masters.fetch_sub(1, Ordering::Release);
        // Wake slave readers (they'll see master gone → HUP).
        self.0.slave_wq.wake_all();
    }
}

impl Inode for PtyMaster {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    /// Master reads get the slave's output (s2m buffer).
    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            loop {
                core::future::poll_fn(|cx| {
                    self.0.master_wq.register_waker(cx.waker());
                    let s2m = self.0.s2m_buf.lock();
                    if !s2m.is_empty() || self.0.slaves.load(Ordering::Acquire) == 0 {
                        core::task::Poll::Ready(())
                    } else {
                        core::task::Poll::Pending
                    }
                })
                .await;

                let mut s2m = self.0.s2m_buf.lock();
                if !s2m.is_empty() {
                    let n = s2m.read(buf);
                    drop(s2m);
                    self.0.slave_wq.wake_one();
                    return Ok(n);
                }
                if self.0.slaves.load(Ordering::Acquire) == 0 {
                    return Ok(0); // EOF — slave hung up.
                }
            }
        })
    }

    /// Master writes go to the slave's input. In canonical mode, the bytes
    /// pass through the line discipline (processing special characters).
    /// In raw mode, they go directly into the m2s buffer.
    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let termios = *self.0.termios.lock();
            let canonical = termios.lflag & hadron_syscall::ICANON != 0;

            if canonical {
                // Canonical mode: feed bytes through the line discipline for
                // line editing (backspace, Enter, Ctrl+C/D handling).
                // Drain completed data into a local buffer to avoid holding
                // the ldisc IrqSpinLock while locking the m2s SpinLock.
                let isig = termios.lflag & hadron_syscall::ISIG != 0;
                let mut staged = [0u8; 512];
                let mut staged_len = 0;
                {
                    let mut ldisc = self.0.ldisc.lock();
                    for &byte in buf {
                        let _ = ldisc.process_ascii_byte(byte, true, isig);
                    }
                    let mut tmp = [0u8; 256];
                    while let Some(n) = ldisc.try_read(&mut tmp) {
                        let end = (staged_len + n).min(staged.len());
                        let copy = end - staged_len;
                        staged[staged_len..end].copy_from_slice(&tmp[..copy]);
                        staged_len = end;
                    }
                }
                // ldisc lock is released — safe to lock m2s now.
                if staged_len > 0 {
                    let mut m2s = self.0.m2s_buf.lock();
                    m2s.write(&staged[..staged_len]);
                }
                self.0.slave_wq.wake_all();
                Ok(buf.len())
            } else {
                // Raw mode: bytes go straight into m2s buffer.
                let mut m2s = self.0.m2s_buf.lock();
                let n = m2s.write(buf);
                drop(m2s);
                self.0.slave_wq.wake_all();
                Ok(n)
            }
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
        use hadron_syscall::{TIOCGPTN, TIOCSPTLCK};

        match cmd {
            TIOCGPTN => {
                let index = self.0.index as u32;
                // SAFETY: arg is a user pointer validated by sys_handle_ioctl.
                unsafe { core::ptr::write_volatile(arg as *mut u32, index) };
                Ok(0)
            }
            TIOCSPTLCK => {
                // SAFETY: arg is a user pointer validated by sys_handle_ioctl.
                let lock_val = unsafe { core::ptr::read(arg as *const u32) };
                self.0.locked.store(lock_val != 0, Ordering::Release);
                Ok(0)
            }
            // Forward termios ioctls to the slave's settings.
            hadron_syscall::TCGETS => {
                let t = *self.0.termios.lock();
                unsafe { core::ptr::write_volatile(arg as *mut Termios, t) };
                Ok(0)
            }
            hadron_syscall::TCSETS | hadron_syscall::TCSETSW | hadron_syscall::TCSETSF => {
                let t = unsafe { core::ptr::read(arg as *const Termios) };
                *self.0.termios.lock() = t;
                Ok(0)
            }
            hadron_syscall::TIOCGWINSZ => {
                let ws = *self.0.winsize.lock();
                unsafe {
                    core::ptr::write_volatile(arg as *mut hadron_syscall::Winsize, ws);
                };
                Ok(0)
            }
            hadron_syscall::TIOCSWINSZ => {
                let ws = unsafe { core::ptr::read(arg as *const hadron_syscall::Winsize) };
                *self.0.winsize.lock() = ws;
                Ok(0)
            }
            _ => Err(FsError::NotSupported),
        }
    }

    fn poll_readiness(&self, waker: Option<&Waker>) -> u16 {
        use hadron_syscall::{POLLHUP, POLLIN, POLLOUT};

        if let Some(w) = waker {
            self.0.master_wq.register_waker(w);
        }
        let s2m = self.0.s2m_buf.lock();
        let mut events = POLLOUT; // Master can always write (feed to slave).
        if !s2m.is_empty() {
            events |= POLLIN;
        }
        if self.0.slaves.load(Ordering::Acquire) == 0 {
            events |= POLLHUP;
            events |= POLLIN; // HUP is readable (read returns 0).
        }
        events
    }
}

// ── Slave side ──────────────────────────────────────────────────────

impl Drop for PtySlave {
    fn drop(&mut self) {
        self.0.slaves.fetch_sub(1, Ordering::Release);
        // Wake master readers (they'll see slave gone → HUP).
        self.0.master_wq.wake_all();
    }
}

impl Inode for PtySlave {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    /// Slave reads get input from the master (m2s buffer).
    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            loop {
                core::future::poll_fn(|cx| {
                    self.0.slave_wq.register_waker(cx.waker());
                    let m2s = self.0.m2s_buf.lock();
                    if !m2s.is_empty() || self.0.masters.load(Ordering::Acquire) == 0 {
                        core::task::Poll::Ready(())
                    } else {
                        core::task::Poll::Pending
                    }
                })
                .await;

                let mut m2s = self.0.m2s_buf.lock();
                if !m2s.is_empty() {
                    let n = m2s.read(buf);
                    drop(m2s);
                    self.0.master_wq.wake_one();
                    return Ok(n);
                }
                if self.0.masters.load(Ordering::Acquire) == 0 {
                    return Ok(0); // EOF — master hung up.
                }
            }
        })
    }

    /// Slave writes go to the master (s2m buffer).
    ///
    /// When `OPOST` is set in the termios output flags, output processing is
    /// applied — in particular `ONLCR` converts `\n` to `\r\n`.
    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.0.masters.load(Ordering::Acquire) == 0 {
                return Err(FsError::BrokenPipe);
            }
            let termios = *self.0.termios.lock();
            let opost = termios.oflag & hadron_syscall::OPOST != 0;
            let onlcr = termios.oflag & hadron_syscall::ONLCR != 0;

            let mut s2m = self.0.s2m_buf.lock();
            if opost && onlcr {
                // Convert \n → \r\n during output.
                for &byte in buf {
                    if byte == b'\n' {
                        s2m.write(&[b'\r', b'\n']);
                    } else {
                        s2m.write(&[byte]);
                    }
                }
            } else {
                s2m.write(buf);
            }
            drop(s2m);
            self.0.master_wq.wake_all();
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
        use hadron_syscall::{
            TCGETS, TCSETS, TCSETSF, TCSETSW, TIOCGPGRP, TIOCGWINSZ, TIOCSPGRP, TIOCSWINSZ,
        };

        match cmd {
            TCGETS => {
                let t = *self.0.termios.lock();
                unsafe { core::ptr::write_volatile(arg as *mut Termios, t) };
                Ok(0)
            }
            TCSETS | TCSETSW | TCSETSF => {
                let t = unsafe { core::ptr::read(arg as *const Termios) };
                *self.0.termios.lock() = t;
                Ok(0)
            }
            TIOCGPGRP => {
                let pgid = self.0.foreground_pgid.load(Ordering::Acquire);
                unsafe { core::ptr::write_volatile(arg as *mut u32, pgid) };
                Ok(0)
            }
            TIOCSPGRP => {
                let pgid = unsafe { core::ptr::read(arg as *const u32) };
                self.0.foreground_pgid.store(pgid, Ordering::Release);
                Ok(0)
            }
            TIOCGWINSZ => {
                let ws = *self.0.winsize.lock();
                unsafe { core::ptr::write_volatile(arg as *mut hadron_syscall::Winsize, ws) };
                Ok(0)
            }
            TIOCSWINSZ => {
                let ws = unsafe { core::ptr::read(arg as *const hadron_syscall::Winsize) };
                *self.0.winsize.lock() = ws;
                Ok(0)
            }
            _ => Err(FsError::NotSupported),
        }
    }

    fn poll_readiness(&self, waker: Option<&Waker>) -> u16 {
        use hadron_syscall::{POLLHUP, POLLIN, POLLOUT};

        if let Some(w) = waker {
            self.0.slave_wq.register_waker(w);
        }
        let m2s = self.0.m2s_buf.lock();
        let mut events = POLLOUT; // Slave can always write (output to master).
        if !m2s.is_empty() {
            events |= POLLIN;
        }
        if self.0.masters.load(Ordering::Acquire) == 0 {
            events |= POLLHUP;
            events |= POLLIN;
        }
        events
    }
}

// ── /dev/ptmx — PTY multiplexer ────────────────────────────────────

/// `/dev/ptmx` — opening this device allocates a new PTY pair.
///
/// The open returns a master fd. The slave is accessible at `/dev/pts/N`
/// where N is obtained via `ioctl(fd, TIOCGPTN, &n)`.
pub struct DevPtmx;

impl Inode for DevPtmx {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        // Reads on /dev/ptmx itself are not meaningful.
        // The actual reading happens on the allocated master fd.
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
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

    /// Opening /dev/ptmx allocates a new PTY pair and returns the master inode.
    fn on_open(&self) -> Result<Option<Arc<dyn Inode>>, FsError> {
        let (master, _slave, _index) = alloc_pty().ok_or(FsError::NotSupported)?;
        Ok(Some(master as Arc<dyn Inode>))
    }
}

// ── /dev/pts/ directory ─────────────────────────────────────────────

/// `/dev/pts/` directory — lists allocated slave PTYs.
pub struct DevPtsDir;

impl Inode for DevPtsDir {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions {
            read: true,
            write: false,
            execute: true,
        }
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let index: usize = name.parse().map_err(|_| FsError::NotFound)?;
            get_slave(index)
                .map(|s| s as Arc<dyn Inode>)
                .ok_or(FsError::NotFound)
        })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async {
            let table = PTY_SLAVES.lock();
            let mut entries = Vec::new();
            for (i, slot) in table.iter().enumerate() {
                if slot.is_some() {
                    let mut name = alloc::string::String::new();
                    use core::fmt::Write;
                    let _ = write!(name, "{i}");
                    entries.push(DirEntry {
                        name,
                        inode_type: InodeType::CharDevice,
                    });
                }
            }
            Ok(entries)
        })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::PermissionDenied) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::PermissionDenied) })
    }
}
