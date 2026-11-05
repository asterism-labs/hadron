use crate::memory::EfiMemoryDescriptor;

/// A parsed UEFI memory map backed by a caller-provided buffer.
pub struct MemoryMap<'buf> {
    buffer: &'buf [u8],
    map_key: usize,
    descriptor_size: usize,
    descriptor_version: u32,
}

impl<'buf> MemoryMap<'buf> {
    /// Create a new `MemoryMap` from the raw output of `GetMemoryMap`.
    pub(crate) fn new(
        buffer: &'buf [u8],
        map_key: usize,
        descriptor_size: usize,
        descriptor_version: u32,
    ) -> Self {
        Self {
            buffer,
            map_key,
            descriptor_size,
            descriptor_version,
        }
    }

    /// The map key, needed for `ExitBootServices`.
    #[must_use]
    pub fn map_key(&self) -> usize {
        self.map_key
    }

    /// Number of descriptors in the map.
    #[must_use]
    pub fn len(&self) -> usize {
        if self.descriptor_size == 0 {
            return 0;
        }
        self.buffer.len() / self.descriptor_size
    }

    /// Returns `true` if the map contains no descriptors.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The firmware-reported descriptor size (may exceed `size_of::<EfiMemoryDescriptor>()`).
    #[must_use]
    pub fn descriptor_size(&self) -> usize {
        self.descriptor_size
    }

    /// The firmware-reported descriptor version.
    #[must_use]
    pub fn descriptor_version(&self) -> u32 {
        self.descriptor_version
    }

    /// Returns an iterator over the memory descriptors.
    #[must_use]
    pub fn iter(&self) -> MemoryMapIter<'buf> {
        MemoryMapIter {
            buffer: self.buffer,
            descriptor_size: self.descriptor_size,
            offset: 0,
        }
    }
}

impl<'buf> IntoIterator for &MemoryMap<'buf> {
    type Item = &'buf EfiMemoryDescriptor;
    type IntoIter = MemoryMapIter<'buf>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over UEFI memory descriptors, stepping by `descriptor_size`.
pub struct MemoryMapIter<'buf> {
    buffer: &'buf [u8],
    descriptor_size: usize,
    offset: usize,
}

impl<'buf> Iterator for MemoryMapIter<'buf> {
    type Item = &'buf EfiMemoryDescriptor;

    fn next(&mut self) -> Option<Self::Item> {
        if self.descriptor_size == 0 || self.offset + self.descriptor_size > self.buffer.len() {
            return None;
        }
        // The alignment is checked below before the pointer is dereferenced.
        #[allow(clippy::cast_ptr_alignment)]
        let ptr = self.buffer[self.offset..]
            .as_ptr()
            .cast::<EfiMemoryDescriptor>();
        if !ptr.is_aligned() {
            return None;
        }
        self.offset += self.descriptor_size;
        // SAFETY: The firmware wrote valid descriptors at this stride, and `ptr` is
        // aligned for `EfiMemoryDescriptor`.
        Some(unsafe { &*ptr })
    }
}
