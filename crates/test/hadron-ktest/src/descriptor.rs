//! Kernel test descriptor types stored in linker sections.

/// Test execution stage — determines when during boot the test runs.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TestStage {
    /// After CPU, HHDM, PMM, VMM, and heap initialization.
    EarlyBoot = 0,
    /// After ACPI, PCI, drivers, VFS, and logging initialization.
    BeforeExecutor = 1,
    /// Inside the async executor (tests are spawned as async tasks).
    WithExecutor = 2,
    /// Full kernel with userspace process support.
    Userspace = 3,
}

impl TestStage {
    /// Returns a human-readable name for the stage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EarlyBoot => "early_boot",
            Self::BeforeExecutor => "before_executor",
            Self::WithExecutor => "with_executor",
            Self::Userspace => "userspace",
        }
    }
}

/// Test function kind — determines how the test runner invokes the function.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    /// Synchronous test: `fn()`.
    Sync = 0,
    /// Async test without instances: wrapper returns `Pin<Box<dyn Future<Output = ()>>>`.
    Async = 1,
    /// Async test with concurrent instances: wrapper takes `&'static TestContext`
    /// and returns `Pin<Box<dyn Future<Output = ()>>>`.
    AsyncInstanced = 2,
}

/// Descriptor for a single kernel test, stored in the `.hadron_kernel_tests`
/// linker section.
///
/// Created by the `#[kernel_test]` proc macro via [`hadron_linkset::linkset_entry!`].
#[repr(C)]
pub struct KernelTestDescriptor {
    /// Test function name (e.g., `"test_heap_alloc"`).
    pub name: &'static str,
    /// Module path where the test is defined (e.g., `"hadron_kernel::mm::tests"`).
    pub module_path: &'static str,
    /// Boot stage at which this test should run.
    pub stage: TestStage,
    /// How to invoke the test function.
    pub kind: TestKind,
    /// First instance ID (0 for non-instanced tests).
    pub instance_start: u32,
    /// Last instance ID, inclusive (0 for non-instanced tests).
    pub instance_end_inclusive: u32,
    /// Per-test watchdog timeout in seconds. 0 = use runner default.
    pub timeout_secs: u32,
    /// Type-erased function pointer. Cast based on `kind`:
    /// - `Sync`: `fn()`
    /// - `Async`: `fn() -> Pin<Box<dyn Future<Output = ()>>>`
    /// - `AsyncInstanced`: `fn(&'static TestContext) -> Pin<Box<dyn Future<Output = ()>>>`
    pub test_fn: *const (),
}

// SAFETY: All fields are either &'static str (Sync+Send), repr(u8) enums
// (Sync+Send), u32 (Sync+Send), or function pointers cast to *const ()
// (function pointers are inherently Sync+Send; the raw pointer wrapper is
// the only reason the auto-trait impl doesn't fire).
unsafe impl Sync for KernelTestDescriptor {}
unsafe impl Send for KernelTestDescriptor {}
