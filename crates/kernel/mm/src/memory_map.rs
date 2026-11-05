//! The physical memory map: what each range of physical memory holds.
//!
//! The kernel copies the bootloader's map into a [`MemoryMap`] because
//! the bootloader's copy lives in memory the kernel reclaims later. The
//! map records only what the firmware and bootloader reported. Which
//! frames are free is the frame allocator's business.

use arch_x86_64::PhysAddr;

/// What a range of physical memory holds. The kinds match the Limine
/// memory map entry types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// Free RAM.
    Usable,
    /// Not usable, and not mapped by the bootloader.
    Reserved,
    /// ACPI tables. Becomes usable once the kernel is done with them.
    AcpiReclaimable,
    /// ACPI non-volatile storage. Never usable.
    AcpiNvs,
    /// Defective RAM.
    BadMemory,
    /// The bootloader's own data, including the page tables, stack and
    /// boot responses. Becomes usable once the kernel no longer needs them.
    BootloaderReclaimable,
    /// The kernel image and the boot modules.
    ExecutableAndModules,
    /// A framebuffer.
    Framebuffer,
    /// Reserved, but mapped by the bootloader: firmware tables and EFI
    /// runtime memory.
    ReservedMapped,
}

impl RegionKind {
    /// Whether the region is RAM that is, or will become, free for the
    /// frame allocator.
    #[must_use]
    pub const fn is_allocatable(self) -> bool {
        matches!(
            self,
            Self::Usable | Self::BootloaderReclaimable | Self::AcpiReclaimable
        )
    }

    /// Whether Limine maps regions of this kind in its direct map, which
    /// is everything except reserved and bad memory.
    #[must_use]
    pub const fn is_direct_mapped(self) -> bool {
        !matches!(self, Self::Reserved | Self::BadMemory)
    }
}

/// A half-open range of physical memory, `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// The first byte.
    pub start: PhysAddr,
    /// One past the last byte.
    pub end: PhysAddr,
    /// What the range holds.
    pub kind: RegionKind,
}

impl Region {
    /// An empty region, used to fill storage before [`MemoryMap::build`].
    pub const EMPTY: Self = Self {
        start: PhysAddr::ZERO,
        end: PhysAddr::ZERO,
        kind: RegionKind::Reserved,
    };

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    /// Whether the region has no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// Why [`MemoryMap::build`] rejected the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// The storage holds fewer regions than the input has.
    TooManyRegions,
    /// Two regions overlap.
    Overlap {
        /// The lower of the two.
        first: Region,
        /// The one that starts inside it.
        second: Region,
    },
}

/// A sorted list of non-overlapping regions. Adjacent regions of the same
/// kind are merged.
#[derive(Debug)]
pub struct MemoryMap<'a> {
    regions: &'a [Region],
}

impl<'a> MemoryMap<'a> {
    /// Builds a map from `regions` in `storage`. The input may be in any
    /// order. Empty regions are dropped.
    ///
    /// # Errors
    ///
    /// Fails if `storage` is too small or two regions overlap.
    pub fn build(
        storage: &'a mut [Region],
        regions: impl IntoIterator<Item = Region>,
    ) -> Result<Self, BuildError> {
        let mut len = 0;
        for region in regions {
            if region.is_empty() {
                continue;
            }
            let slot = storage.get_mut(len).ok_or(BuildError::TooManyRegions)?;
            *slot = region;
            len += 1;
        }
        let storage = &mut storage[..len];
        storage.sort_unstable_by_key(|region| region.start);

        let mut merged = 0;
        for i in 0..storage.len() {
            let region = storage[i];
            if merged > 0 {
                let last = &mut storage[merged - 1];
                if region.start < last.end {
                    return Err(BuildError::Overlap {
                        first: *last,
                        second: region,
                    });
                }
                if region.start == last.end && region.kind == last.kind {
                    last.end = region.end;
                    continue;
                }
            }
            storage[merged] = region;
            merged += 1;
        }
        Ok(Self {
            regions: &storage[..merged],
        })
    }

    /// The regions, lowest first.
    #[must_use]
    pub const fn regions(&self) -> &'a [Region] {
        self.regions
    }

    /// The regions of `kind`, lowest first.
    pub fn of_kind(&self, kind: RegionKind) -> impl Iterator<Item = &'a Region> + 'a {
        self.regions
            .iter()
            .filter(move |region| region.kind == kind)
    }

    /// Total bytes in regions of `kind`.
    #[must_use]
    pub fn total(&self, kind: RegionKind) -> u64 {
        self.of_kind(kind).map(Region::len).sum()
    }

    /// One past the highest byte of any allocatable region, or 0 if there
    /// is none.
    #[must_use]
    pub fn allocatable_end(&self) -> PhysAddr {
        self.regions
            .iter()
            .filter(|region| region.kind.is_allocatable())
            .map(|region| region.end)
            .max()
            .unwrap_or(PhysAddr::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(start: u64, end: u64, kind: RegionKind) -> Region {
        Region {
            start: PhysAddr::new(start),
            end: PhysAddr::new(end),
            kind,
        }
    }

    #[test]
    fn sorts_merges_and_drops_empty_regions() {
        let mut storage = [Region::EMPTY; 8];
        let input = [
            region(0x3000, 0x4000, RegionKind::Usable),
            region(0x1000, 0x2000, RegionKind::Usable),
            region(0x2000, 0x3000, RegionKind::Usable),
            region(0x5000, 0x5000, RegionKind::Reserved),
            region(0x4000, 0x6000, RegionKind::AcpiNvs),
        ];
        let map = MemoryMap::build(&mut storage, input).unwrap();
        assert_eq!(
            map.regions(),
            &[
                region(0x1000, 0x4000, RegionKind::Usable),
                region(0x4000, 0x6000, RegionKind::AcpiNvs),
            ]
        );
    }

    #[test]
    fn rejects_overlap() {
        let mut storage = [Region::EMPTY; 4];
        let input = [
            region(0x1000, 0x3000, RegionKind::Usable),
            region(0x2000, 0x4000, RegionKind::Reserved),
        ];
        assert_eq!(
            MemoryMap::build(&mut storage, input).unwrap_err(),
            BuildError::Overlap {
                first: input[0],
                second: input[1],
            }
        );
    }

    #[test]
    fn rejects_too_many_regions() {
        let mut storage = [Region::EMPTY; 1];
        let input = [
            region(0x1000, 0x2000, RegionKind::Usable),
            region(0x3000, 0x4000, RegionKind::Usable),
        ];
        assert_eq!(
            MemoryMap::build(&mut storage, input).unwrap_err(),
            BuildError::TooManyRegions
        );
    }

    #[test]
    fn totals_and_allocatable_end() {
        let mut storage = [Region::EMPTY; 4];
        let input = [
            region(0x0, 0x9_f000, RegionKind::Usable),
            region(0x10_0000, 0x20_0000, RegionKind::BootloaderReclaimable),
            region(0xfd00_0000, 0xfe00_0000, RegionKind::Framebuffer),
        ];
        let map = MemoryMap::build(&mut storage, input).unwrap();
        assert_eq!(map.total(RegionKind::Usable), 0x9_f000);
        assert_eq!(map.allocatable_end(), PhysAddr::new(0x20_0000));
    }
}
