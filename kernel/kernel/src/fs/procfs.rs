//! Process filesystem (`/proc`) compatibility shim.
//!
//! Provides a minimal subset of `/proc` that Mesa and musl require:
//! - `/proc/self` — magic symlink to `/proc/<current_pid>`
//! - `/proc/meminfo` — PMM statistics in Linux format
//! - `/proc/cpuinfo` — CPU vendor + feature flags in Linux format
//! - `/proc/<pid>/maps` — VMA dump for address space layout
//! - `/proc/<pid>/exe` — symlink to the process executable path
//! - `/proc/<pid>/status` — name, pid, ppid in Linux format
//!
//! All file contents are generated fresh on every `read()` call; there is no
//! snapshot caching. `size()` returns 0 (matching Linux procfs convention).

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use crate::fs::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};
use crate::id::Pid;
use crate::proc::{MappingKind, ProcessTable};

// ── ProcFs ──────────────────────────────────────────────────────────────

/// The procfs filesystem.
pub struct ProcFs {
    root: Arc<ProcRootDir>,
}

impl ProcFs {
    /// Create a new procfs instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: Arc::new(ProcRootDir),
        }
    }
}

impl Default for ProcFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for ProcFs {
    fn name(&self) -> &'static str {
        "procfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

// ── ProcRootDir ─────────────────────────────────────────────────────────

/// The `/proc` root directory.
struct ProcRootDir;

impl Inode for ProcRootDir {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async move {
            match name {
                "self" => Ok(Arc::new(ProcSelfLink) as Arc<dyn Inode>),
                "meminfo" => Ok(Arc::new(ProcGlobalFile {
                    generator: gen_meminfo,
                }) as Arc<dyn Inode>),
                "cpuinfo" => Ok(Arc::new(ProcGlobalFile {
                    generator: gen_cpuinfo,
                }) as Arc<dyn Inode>),
                other => {
                    // Try to parse as a PID.
                    let pid: u32 = other.parse().map_err(|_| FsError::NotFound)?;
                    let pid = Pid::new(pid);
                    if ProcessTable::lookup(pid).is_some() {
                        Ok(Arc::new(ProcPidDir { pid }) as Arc<dyn Inode>)
                    } else {
                        Err(FsError::NotFound)
                    }
                }
            }
        })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async move {
            let mut entries = alloc::vec![
                DirEntry {
                    name: "self".into(),
                    inode_type: InodeType::Symlink,
                },
                DirEntry {
                    name: "meminfo".into(),
                    inode_type: InodeType::File,
                },
                DirEntry {
                    name: "cpuinfo".into(),
                    inode_type: InodeType::File,
                },
            ];
            for pid in ProcessTable::all_pids() {
                entries.push(DirEntry {
                    name: pid.as_u32().to_string(),
                    inode_type: InodeType::Directory,
                });
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
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }
}

// ── ProcSelfLink ────────────────────────────────────────────────────────

/// `/proc/self` — symlink to `/proc/<current_pid>`.
struct ProcSelfLink;

impl Inode for ProcSelfLink {
    fn inode_type(&self) -> InodeType {
        InodeType::Symlink
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read_link(&self) -> Result<String, FsError> {
        let pid = ProcessTable::try_current(|p| p.pid);
        if let Some(pid) = pid {
            Ok(format!("/proc/{}", pid.as_u32()))
        } else {
            Err(FsError::NotFound)
        }
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Ok(0) })
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
}

// ── ProcGlobalFile ──────────────────────────────────────────────────────

/// A procfs file whose content is generated by a closure on each read.
struct ProcGlobalFile {
    generator: fn() -> Vec<u8>,
}

impl Inode for ProcGlobalFile {
    fn inode_type(&self) -> InodeType {
        InodeType::File
    }

    fn size(&self) -> usize {
        // Linux procfs convention: report 0 for dynamic files.
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        let content = (self.generator)();
        Box::pin(async move {
            if offset >= content.len() {
                return Ok(0);
            }
            let available = &content[offset..];
            let n = available.len().min(buf.len());
            buf[..n].copy_from_slice(&available[..n]);
            Ok(n)
        })
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
}

// ── ProcPidDir ──────────────────────────────────────────────────────────

/// `/proc/<pid>` — per-process directory.
struct ProcPidDir {
    pid: Pid,
}

impl Inode for ProcPidDir {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        let pid = self.pid;
        Box::pin(async move {
            // Verify the process still exists.
            if ProcessTable::lookup(pid).is_none() {
                return Err(FsError::NotFound);
            }
            match name {
                "maps" => Ok(Arc::new(ProcPidFile {
                    pid,
                    generator: gen_maps,
                }) as Arc<dyn Inode>),
                "exe" => Ok(Arc::new(ProcExeLink { pid }) as Arc<dyn Inode>),
                "status" => Ok(Arc::new(ProcPidFile {
                    pid,
                    generator: gen_status,
                }) as Arc<dyn Inode>),
                _ => Err(FsError::NotFound),
            }
        })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async {
            Ok(alloc::vec![
                DirEntry {
                    name: "maps".into(),
                    inode_type: InodeType::File,
                },
                DirEntry {
                    name: "exe".into(),
                    inode_type: InodeType::Symlink,
                },
                DirEntry {
                    name: "status".into(),
                    inode_type: InodeType::File,
                },
            ])
        })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }
}

// ── ProcPidFile ─────────────────────────────────────────────────────────

/// A per-process procfs file generated from the live process state.
struct ProcPidFile {
    pid: Pid,
    generator: fn(Pid) -> Vec<u8>,
}

impl Inode for ProcPidFile {
    fn inode_type(&self) -> InodeType {
        InodeType::File
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        let content = (self.generator)(self.pid);
        Box::pin(async move {
            if offset >= content.len() {
                return Ok(0);
            }
            let available = &content[offset..];
            let n = available.len().min(buf.len());
            buf[..n].copy_from_slice(&available[..n]);
            Ok(n)
        })
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
}

// ── ProcExeLink ─────────────────────────────────────────────────────────

/// `/proc/<pid>/exe` — symlink to the process executable.
struct ProcExeLink {
    pid: Pid,
}

impl Inode for ProcExeLink {
    fn inode_type(&self) -> InodeType {
        InodeType::Symlink
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read_link(&self) -> Result<String, FsError> {
        ProcessTable::lookup(self.pid)
            .map(|p| p.exe_path.lock().clone())
            .ok_or(FsError::NotFound)
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Ok(0) })
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
}

// ── Content generators ──────────────────────────────────────────────────

/// Generate `/proc/meminfo` content.
fn gen_meminfo() -> Vec<u8> {
    let (total, free) = crate::mm::pmm::with(|pmm| (pmm.total_frames(), pmm.free_frames()));
    // 4 KiB per frame, convert to kB.
    let total_kb = total * 4;
    let free_kb = free * 4;
    format!(
        "MemTotal:       {} kB\nMemFree:        {} kB\nMemAvailable:   {} kB\n",
        total_kb, free_kb, free_kb,
    )
    .into_bytes()
}

/// Generate `/proc/cpuinfo` content.
fn gen_cpuinfo() -> Vec<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        use crate::arch::x86_64::cpuid::{CpuFeatures, cpu_features, cpuid};

        // Read vendor string from CPUID leaf 0: EBX, EDX, ECX (in that order).
        let leaf0 = cpuid(0);
        let vendor = {
            let mut bytes = [0u8; 12];
            bytes[0..4].copy_from_slice(&leaf0.ebx.to_le_bytes());
            bytes[4..8].copy_from_slice(&leaf0.edx.to_le_bytes());
            bytes[8..12].copy_from_slice(&leaf0.ecx.to_le_bytes());
            core::str::from_utf8(&bytes)
                .unwrap_or("Unknown")
                .trim_matches('\0')
                .to_string()
        };

        let features = cpu_features();
        let mut flags = alloc::vec::Vec::<&str>::new();
        // x86_64 baseline features (always present).
        flags.push("fpu");
        if features.contains(CpuFeatures::SSE2) {
            flags.push("sse2");
        }
        if features.contains(CpuFeatures::SSE3) {
            flags.push("sse3");
        }
        if features.contains(CpuFeatures::SSSE3) {
            flags.push("ssse3");
        }
        if features.contains(CpuFeatures::SSE4_1) {
            flags.push("sse4_1");
        }
        if features.contains(CpuFeatures::SSE4_2) {
            flags.push("sse4_2");
        }
        if features.contains(CpuFeatures::AVX) {
            flags.push("avx");
        }
        if features.contains(CpuFeatures::AVX2) {
            flags.push("avx2");
        }
        if features.contains(CpuFeatures::POPCNT) {
            flags.push("popcnt");
        }
        if features.contains(CpuFeatures::BMI1) {
            flags.push("bmi1");
        }
        if features.contains(CpuFeatures::BMI2) {
            flags.push("bmi2");
        }
        if features.contains(CpuFeatures::ERMS) {
            flags.push("erms");
        }

        format!(
            "processor\t: 0\nvendor_id\t: {}\nmodel name\t: Hadron x86_64\nflags\t\t: {}\n",
            vendor,
            flags.join(" "),
        )
        .into_bytes()
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        b"processor\t: 0\nmodel name\t: Hadron\n".to_vec()
    }
}

/// Generate `/proc/<pid>/maps` content.
fn gen_maps(pid: Pid) -> Vec<u8> {
    let process = match ProcessTable::lookup(pid) {
        Some(p) => p,
        None => return alloc::vec![],
    };

    let mappings = process.mmap_mappings.lock();
    let mut out = String::new();
    for (&start, kind) in mappings.iter() {
        let (page_count, perms) = match *kind {
            MappingKind::Anonymous { page_count } => (page_count, "rw-p"),
            MappingKind::Device { page_count } => (page_count, "r--s"),
            MappingKind::Shared { page_count } => (page_count, "rw-s"),
        };
        let end = start + (page_count as u64) * 4096;
        // Format: <start>-<end> <perms> 00000000 00:00 0
        let line = format!("{:016x}-{:016x} {} 00000000 00:00 0\n", start, end, perms);
        out.push_str(&line);
    }
    out.into_bytes()
}

/// Generate `/proc/<pid>/status` content.
fn gen_status(pid: Pid) -> Vec<u8> {
    let process = match ProcessTable::lookup(pid) {
        Some(p) => p,
        None => return alloc::vec![],
    };

    let exe = process.exe_path.lock().clone();
    // Extract the basename from the exe path.
    let name = exe.rsplit('/').next().unwrap_or(&exe);
    let ppid = process.parent_pid.map_or(0, |p| p.as_u32());

    format!(
        "Name:\t{}\nPid:\t{}\nPPid:\t{}\nVmRSS:\t0 kB\n",
        name,
        pid.as_u32(),
        ppid,
    )
    .into_bytes()
}
