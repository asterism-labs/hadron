//! Handle table and rights system.
//!
//! Each process owns a [`HandleTable`] mapping integer handle values to
//! [`HandleEntry`] records. A handle entry pairs an `Arc<dyn KernelObject>`
//! with a [`Rights`] mask that restricts what operations the holder may perform.
//!
//! Rights can only be reduced (via [`HandleTable::duplicate`]), never amplified.

use alloc::{collections::BTreeMap, sync::Arc};

use bitflags::bitflags;

use crate::object::KernelObject;

/// An opaque integer identifying a handle within a process.
///
/// Handle values are process-local — the same value in two different processes
/// refers to unrelated objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct HandleValue(u32);

impl HandleValue {
    /// The invalid/null handle value.
    pub const INVALID: Self = Self(0);

    /// Create a handle value from a raw `u32`.
    #[must_use]
    pub const fn from_raw(v: u32) -> Self {
        Self(v)
    }

    /// Return the raw `u32` value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

bitflags! {
    /// Access rights associated with a handle.
    ///
    /// When a handle is duplicated, the caller may specify a subset of the
    /// original rights. Rights can never be amplified — this is a fundamental
    /// security invariant.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Rights: u32 {
        /// Read data from the object (channel messages, VMO bytes, etc.).
        const READ           = 1 << 0;
        /// Write data to the object.
        const WRITE          = 1 << 1;
        /// Execute code from the object (VMO → process mapping).
        const EXECUTE        = 1 << 2;
        /// Map the object into an address space (VMO → VMAR).
        const MAP            = 1 << 3;
        /// Duplicate the handle (possibly with reduced rights).
        const DUPLICATE      = 1 << 4;
        /// Transfer the handle over a channel.
        const TRANSFER       = 1 << 5;
        /// Raise or clear user-visible signals on the object.
        const SIGNAL         = 1 << 6;
        /// Wait on the object's signals.
        const WAIT           = 1 << 7;
        /// Manage processes (start, kill).
        const MANAGE_PROCESS = 1 << 8;
        /// Manage threads (start, suspend, kill).
        const MANAGE_THREAD  = 1 << 9;
        /// Enumerate children (job → processes, process → threads).
        const ENUMERATE      = 1 << 10;
        /// Set policy on jobs or resources.
        const SET_POLICY     = 1 << 11;

        /// All rights — used for the initial handle to a newly created object.
        const ALL = Self::READ.bits()
            | Self::WRITE.bits()
            | Self::EXECUTE.bits()
            | Self::MAP.bits()
            | Self::DUPLICATE.bits()
            | Self::TRANSFER.bits()
            | Self::SIGNAL.bits()
            | Self::WAIT.bits()
            | Self::MANAGE_PROCESS.bits()
            | Self::MANAGE_THREAD.bits()
            | Self::ENUMERATE.bits()
            | Self::SET_POLICY.bits();

        /// Default rights for a newly created channel endpoint.
        const CHANNEL_DEFAULT = Self::READ.bits()
            | Self::WRITE.bits()
            | Self::DUPLICATE.bits()
            | Self::TRANSFER.bits()
            | Self::SIGNAL.bits()
            | Self::WAIT.bits();

        /// Default rights for a newly created VMO.
        const VMO_DEFAULT = Self::READ.bits()
            | Self::WRITE.bits()
            | Self::MAP.bits()
            | Self::DUPLICATE.bits()
            | Self::TRANSFER.bits()
            | Self::WAIT.bits();
    }
}

/// A single entry in a process's handle table.
pub struct HandleEntry {
    /// The referenced kernel object.
    object: Arc<dyn KernelObject>,
    /// The rights mask for this handle.
    rights: Rights,
}

impl core::fmt::Debug for HandleEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HandleEntry")
            .field("object_type", &self.object.object_type())
            .field("koid", &self.object.koid())
            .field("rights", &self.rights)
            .finish()
    }
}

impl HandleEntry {
    /// Create a new handle entry with the given object and rights.
    #[must_use]
    pub fn new(object: Arc<dyn KernelObject>, rights: Rights) -> Self {
        Self { object, rights }
    }

    /// The kernel object referenced by this handle.
    #[must_use]
    pub fn object(&self) -> &Arc<dyn KernelObject> {
        &self.object
    }

    /// The rights associated with this handle.
    #[must_use]
    pub fn rights(&self) -> Rights {
        self.rights
    }
}

/// Error returned by handle table operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleError {
    /// The handle value does not exist in the table.
    NotFound,
    /// The handle does not have the required rights for this operation.
    AccessDenied,
    /// The handle table is full (too many open handles).
    TableFull,
}

/// Per-process handle table mapping [`HandleValue`] → [`HandleEntry`].
///
/// The handle table is the sole mechanism by which userspace references kernel
/// objects. Every syscall that operates on an object takes a [`HandleValue`]
/// and the table validates both existence and rights.
pub struct HandleTable {
    /// Sparse map from handle values to entries.
    entries: BTreeMap<HandleValue, HandleEntry>,
    /// Monotonically increasing counter for the next handle value.
    ///
    /// Starts at 1 because [`HandleValue::INVALID`] (0) is reserved.
    next_value: u32,
}

impl HandleTable {
    /// Maximum number of handles a single process may hold.
    const MAX_HANDLES: usize = 1 << 16;

    /// Create an empty handle table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            next_value: 1,
        }
    }

    /// Insert an object into the table, returning its new handle value.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::TableFull`] if the table has reached its
    /// capacity limit.
    pub fn insert(&mut self, entry: HandleEntry) -> Result<HandleValue, HandleError> {
        if self.entries.len() >= Self::MAX_HANDLES {
            return Err(HandleError::TableFull);
        }
        let value = HandleValue(self.next_value);
        self.next_value = self.next_value.wrapping_add(1);
        // Skip INVALID (0) on wrap.
        if self.next_value == 0 {
            self.next_value = 1;
        }
        self.entries.insert(value, entry);
        Ok(value)
    }

    /// Remove a handle from the table, returning its entry.
    ///
    /// This is the `handle_close` operation — the returned entry's `Arc` will
    /// be dropped, potentially destroying the underlying object if this was the
    /// last reference.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::NotFound`] if the handle does not exist.
    pub fn remove(&mut self, value: HandleValue) -> Result<HandleEntry, HandleError> {
        self.entries.remove(&value).ok_or(HandleError::NotFound)
    }

    /// Look up a handle entry by value.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::NotFound`] if the handle does not exist.
    pub fn get(&self, value: HandleValue) -> Result<&HandleEntry, HandleError> {
        self.entries.get(&value).ok_or(HandleError::NotFound)
    }

    /// Look up a handle and verify it has the required rights.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::NotFound`] if the handle does not exist, or
    /// [`HandleError::AccessDenied`] if the handle lacks the required rights.
    pub fn get_with_rights(
        &self,
        value: HandleValue,
        required: Rights,
    ) -> Result<&HandleEntry, HandleError> {
        let entry = self.get(value)?;
        if entry.rights.contains(required) {
            Ok(entry)
        } else {
            Err(HandleError::AccessDenied)
        }
    }

    /// Duplicate a handle with equal or reduced rights.
    ///
    /// The new handle refers to the same underlying object but may have fewer
    /// rights. The original handle is not modified.
    ///
    /// # Errors
    ///
    /// - [`HandleError::NotFound`] if the source handle does not exist.
    /// - [`HandleError::AccessDenied`] if the source handle lacks
    ///   [`Rights::DUPLICATE`], or if `new_rights` is not a subset of the
    ///   source handle's rights.
    /// - [`HandleError::TableFull`] if the table is at capacity.
    pub fn duplicate(
        &mut self,
        value: HandleValue,
        new_rights: Rights,
    ) -> Result<HandleValue, HandleError> {
        let entry = self.entries.get(&value).ok_or(HandleError::NotFound)?;

        // Must have DUPLICATE right on the source handle.
        if !entry.rights.contains(Rights::DUPLICATE) {
            return Err(HandleError::AccessDenied);
        }

        // New rights must be a subset of existing rights.
        if !entry.rights.contains(new_rights) {
            return Err(HandleError::AccessDenied);
        }

        let new_entry = HandleEntry::new(Arc::clone(&entry.object), new_rights);
        self.insert(new_entry)
    }

    /// The number of handles currently in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the handle table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for HandleTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use core::sync::atomic::AtomicU32;

    use super::*;
    use crate::object::{Koid, ObjectType, Signals};

    /// Minimal test object for handle table tests.
    struct TestObject {
        koid: Koid,
        signals: AtomicU32,
    }

    impl TestObject {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                koid: Koid::alloc(),
                signals: AtomicU32::new(0),
            })
        }
    }

    impl KernelObject for TestObject {
        fn object_type(&self) -> ObjectType {
            ObjectType::Event
        }

        fn koid(&self) -> Koid {
            self.koid
        }

        fn get_signals(&self) -> Signals {
            Signals::from_bits_truncate(self.signals.load(core::sync::atomic::Ordering::Relaxed))
        }

        fn add_observer(
            &self,
            _port: Arc<dyn crate::observer::PortDispatch>,
            _key: u64,
            _signals: Signals,
        ) {
        }

        fn remove_observer(&self, _port: &Arc<dyn crate::observer::PortDispatch>) {}
    }

    #[test]
    fn insert_and_get() {
        let mut table = HandleTable::new();
        let obj = TestObject::new();
        let koid = obj.koid();

        let hv = table
            .insert(HandleEntry::new(obj, Rights::ALL))
            .expect("insert failed");
        assert_ne!(hv, HandleValue::INVALID);

        let entry = table.get(hv).expect("get failed");
        assert_eq!(entry.object().koid(), koid);
        assert_eq!(entry.rights(), Rights::ALL);
    }

    #[test]
    fn remove_returns_entry() {
        let mut table = HandleTable::new();
        let obj = TestObject::new();
        let koid = obj.koid();

        let hv = table
            .insert(HandleEntry::new(obj, Rights::ALL))
            .expect("insert failed");

        let entry = table.remove(hv).expect("remove failed");
        assert_eq!(entry.object().koid(), koid);

        // Second remove should fail.
        assert!(matches!(table.remove(hv), Err(HandleError::NotFound)));
    }

    #[test]
    fn get_with_rights_checks() {
        let mut table = HandleTable::new();
        let obj = TestObject::new();

        let hv = table
            .insert(HandleEntry::new(obj, Rights::READ | Rights::WAIT))
            .expect("insert failed");

        // Requesting a subset succeeds.
        assert!(table.get_with_rights(hv, Rights::READ).is_ok());

        // Requesting a right not held fails.
        assert!(matches!(
            table.get_with_rights(hv, Rights::WRITE),
            Err(HandleError::AccessDenied),
        ));
    }

    #[test]
    fn duplicate_reduces_rights() {
        let mut table = HandleTable::new();
        let obj = TestObject::new();
        let koid = obj.koid();

        let hv = table
            .insert(HandleEntry::new(obj, Rights::ALL))
            .expect("insert failed");

        let hv2 = table
            .duplicate(hv, Rights::READ | Rights::WAIT)
            .expect("duplicate failed");

        let entry2 = table.get(hv2).expect("get duplicate failed");
        assert_eq!(entry2.object().koid(), koid);
        assert_eq!(entry2.rights(), Rights::READ | Rights::WAIT);
    }

    #[test]
    fn duplicate_cannot_amplify_rights() {
        let mut table = HandleTable::new();
        let obj = TestObject::new();

        let hv = table
            .insert(HandleEntry::new(obj, Rights::READ | Rights::DUPLICATE))
            .expect("insert failed");

        // Requesting WRITE (not in source rights) should fail.
        assert!(matches!(
            table.duplicate(hv, Rights::READ | Rights::WRITE),
            Err(HandleError::AccessDenied),
        ));
    }

    #[test]
    fn duplicate_requires_duplicate_right() {
        let mut table = HandleTable::new();
        let obj = TestObject::new();

        // Handle without DUPLICATE right.
        let hv = table
            .insert(HandleEntry::new(obj, Rights::READ))
            .expect("insert failed");

        assert!(matches!(
            table.duplicate(hv, Rights::READ),
            Err(HandleError::AccessDenied),
        ));
    }
}
