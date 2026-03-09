# IOMMU and Device Isolation

Hadron uses Intel VT-d (Virtualization Technology for Directed I/O) to restrict what physical
memory each device can access via DMA. Without IOMMU protection, a compromised device driver could
program a device to DMA-read kernel memory, extract private data, or overwrite page tables. With
IOMMU protection, each device operates within a strictly bounded physical address domain, and any
DMA access outside that domain generates a fault that the kernel can log and handle.

---

## DMAR Table Parsing

The ACPI DMAR (DMA Remapping) table is parsed during kernel bootstrap (step 14 of the boot
procedure). It describes the remapping hardware units present on the platform:

- **DRHD (DMA Remapping Hardware Definition)** — each DRHD record identifies a VT-d unit's MMIO
  base address and the PCI buses it covers. A DRHD with the `INCLUDE_ALL` flag covers all buses
  not listed in other DRHD records.
- **ATSR (ATS Root) / RHSA (Remapping Hardware Status)** — additional topology records consumed
  but not critical to the basic isolation model.

For each DRHD, the kernel:

1. Maps the unit's MMIO registers into the kernel MMIO virtual address region via the MMIO mapper.
2. Programs the root table: a 4 KiB page indexed by PCI bus number. Each bus entry points to a
   context table.
3. Programs the context table: one entry per (device, function) pair on that bus. Each context
   entry holds the address of a second-level page table (the DMA domain) and a domain ID.
4. Sets the default translation entry to an empty domain (no mappings). Any DMA from a device not
   yet assigned a domain will fault immediately.
5. Enables address translation in the unit's global command register.

---

## Kernel Objects

The IOMMU subsystem exposes three kernel object types:

### Iommu

An `Iommu` object represents one VT-d remapping hardware unit. It owns the root and context tables
and the domain ID allocator. A handle to an `Iommu` object is required to create `Bti` objects for
devices covered by that unit.

The root resource holds handles to all `Iommu` objects. `devmgr` receives these handles and
distributes them to driver processes.

### Bti (Bus Transaction Initiator)

A `Bti` object represents one device's DMA permission domain. It corresponds to one context table
entry in the VT-d hardware.

Creating a `Bti` requires:
- An `Iommu` handle (to identify which unit owns the device).
- The PCI BDF (bus/device/function) of the device.

On creation, the kernel allocates a domain ID, creates a second-level page table for that domain,
and writes the context entry pointing to it. Initially the domain is empty — no physical pages
are accessible.

### Pmt (Pinned Memory Token)

A `Pmt` represents a single DMA transaction: a VMO region that has been pinned in physical memory
and mapped into a `Bti`'s domain. Creating a `Pmt` is what actually allows a device to perform
DMA.

A `Pmt` holds:
- A reference to the `Bti` it belongs to.
- The list of physical addresses (page frames) that were pinned.
- The IOMMU virtual address range (device-visible) assigned to this mapping.
- The access permissions (`READ`, `WRITE`, or both).

When a `Pmt` is unpinned (`pmt_unpin`), the kernel removes the mapping from the `Bti`'s second-
level page table and issues a TLB invalidation to the VT-d unit. The physical pages are then free
to be reclaimed or remapped.

---

## Driver DMA Flow

The full flow for a driver performing DMA:

```mermaid
sequenceDiagram
    participant Driver as Driver Process
    participant K as Kernel (IOMMU subsystem)
    participant HW as VT-d Hardware

    Driver->>K: vmo_create_contiguous(size) → dma_vmo
    Note over K: Allocate physically contiguous pages, return VMO fd
    Driver->>K: bti_pin(bti_fd, dma_vmo, perm=READ|WRITE) → pmt_fd + phys_addrs[]
    Note over K: Pin VMO pages in physical memory (non-swappable)
    K->>HW: Program second-level page table entry: iova → phys
    K->>HW: Flush IOMMU TLB for domain
    K-->>Driver: pmt_fd, phys_addrs array
    Driver->>Driver: Write phys_addrs[] to device DMA descriptor registers
    Driver->>Driver: Ring doorbell / issue command to device
    Note over HW: Device performs DMA via IOMMU → allowed range only
    Driver->>K: pmt_unpin(pmt_fd)
    K->>HW: Remove second-level page table entry
    K->>HW: Flush IOMMU TLB for domain
    Note over K: VMO pages are unpinned; driver must not use phys_addrs again
```

The kernel validates the `perm` argument against the rights on the `Bti` handle. A driver that
holds a read-only `Bti` cannot pin pages for write access. This prevents a compromised driver from
using DMA to overwrite kernel memory even if it can craft arbitrary DMA descriptors.

### Physical Address Array

`bti_pin` returns an array of physical page frame addresses (one per page). For a contiguous VMO,
this array has a single entry. For a non-contiguous VMO, it may have one entry per page. The driver
is responsible for configuring scatter-gather lists appropriately.

### Quarantine

When a device emits a DMA fault (detected by the VT-d fault interrupt), the kernel moves the
device's context entry to the quarantine domain — an isolated domain that maps a single fault page
but nothing else. This prevents cascading faults while still allowing the hardware to respond to
bus transactions. The `bti_release_quarantine` syscall (design) removes the quarantine and allows
the driver to re-initialize the device.

---

## Security Guarantee

The IOMMU isolation guarantee is: **a compromised driver cannot DMA to any physical address that
the kernel has not explicitly allowed.**

More precisely:

1. Every DMA access by a device passes through the VT-d hardware's second-level address
   translation.
2. Only addresses explicitly mapped into the device's `Bti` domain are accessible.
3. Mappings are created only by the kernel's `bti_pin` path, which validates both the caller's
   rights and the target VMO's properties.
4. Kernel memory (the HHDM mapping, kernel image, heap, stacks) is never mapped into any `Bti`
   domain. A compromised driver cannot read kernel page tables or code.
5. One device's domain is completely separate from another device's domain. A compromised NIC
   driver cannot DMA into a storage device's buffers.

This design means the threat model for drivers is significantly weaker than on a system without
IOMMU. A buggy or malicious driver can corrupt its own address space and the memory it was
explicitly given DMA access to, but cannot escape its sandbox through hardware.

---

## MMIO Access (Separate from DMA)

IOMMU protects DMA (device-initiated memory accesses). CPU-initiated MMIO accesses — the driver
reading and writing device registers — are governed by a different mechanism: the `MmioFrame` VMO.

A `MmioFrame` VMO is a kernel object wrapping a physical MMIO range. The driver maps it with
`vmar_map(..., Uncacheable)` and accesses device registers through the resulting virtual address.
The kernel validates the MMIO range against the device's PCI BAR before creating the VMO, ensuring
the driver can only access registers belonging to its own device.

---

## Syscall Reference

| Syscall (design) | Arguments | Description |
|-----------------|-----------|-------------|
| `bti_create` | `iommu_fd`, `bdf` | Create a `Bti` for a PCI device. Returns a `Bti` fd. |
| `bti_pin` | `bti_fd`, `vmo_fd`, `perm`, `phys_out_ptr`, `phys_out_len` | Pin VMO pages and map them into the IOMMU domain. Returns a `Pmt` fd and the physical address array. |
| `bti_release_quarantine` | `bti_fd` | Release the device from quarantine after a DMA fault. |
| `pmt_unpin` | `pmt_fd` | Remove the IOMMU mapping and unpin the VMO pages. |
