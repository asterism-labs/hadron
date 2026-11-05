# hadron-acpi

A standalone, `no_std` ACPI table parser for the Hadron kernel. This crate provides types and iterators for parsing the core ACPI tables needed during early boot --- RSDP, RSDT/XSDT, MADT, FADT, HPET, MCFG, SRAT, SLIT, DMAR, IVRS, and BGRT --- as well as an AML bytecode walker for extracting the ACPI namespace from DSDT/SSDT tables. All table iteration is done through safe byte-slice iterators backed by an `AcpiHandler` trait that maps physical memory on demand. The crate does not depend on `alloc` by default; enable the `alloc` feature to use the `NamespaceBuilder` which collects namespace nodes into a `Vec`.

## Features

- **Full RSDP/RSDT/XSDT discovery** --- validates ACPI 1.0 and 2.0+ root pointers and iterates over all table entries
- **MADT parsing** --- enumerates local APICs, I/O APICs, interrupt source overrides, NMI sources, and local APIC NMIs via a `TableEntries`-derived iterator
- **FADT parsing** --- extracts PM timer port, boot architecture flags, feature flags, and DSDT/FACS addresses (with 32/64-bit fallback)
- **MCFG (PCIe ECAM) parsing** --- iterates PCI segment group ECAM base addresses and bus ranges
- **HPET table parsing** --- reads the timer block ID, base address, and minimum tick
- **BGRT table parsing** --- reads the boot logo image type, physical address, and screen coordinates
- **NUMA topology (SRAT + SLIT)** --- parses processor/memory affinity entries and the inter-node distance matrix
- **IOMMU tables (DMAR + IVRS)** --- parses Intel VT-d DRHDs/RMRRs/ATSRs and AMD-Vi IVHDs/IVMDs with device scope iteration
- **AML namespace walker** --- single-pass bytecode parser that extracts devices, scopes, methods, thermal zones, processors, and power resources via the `AmlVisitor` trait
- **AML value resolution** --- resolves integer constants, EISA IDs, inline strings, and resource template buffers from `DefName` objects
- **Resource template parser** --- decodes `_CRS`/`_PRS` descriptors including I/O ports, IRQs, DMA channels, memory ranges (32/64-bit), and extended IRQs
- **PCI routing table (`_PRT`) extraction** --- parses hardwired GSI routing entries from `_PRT` packages
- **`NamespaceBuilder` (alloc)** --- collects the full namespace into a searchable tree with device lookup by `_HID` (EISA ID or string)

## Architecture

The crate is organized around an `AcpiTables` entry point that holds an `AcpiHandler` and lazily parses individual tables on request. Each table type lives in its own module (`madt`, `fadt`, `mcfg`, `hpet`, `bgrt`, `dmar`, `ivrs`, `srat`, `slit`) and follows a common pattern: `load_table` maps and validates the SDT header and checksum, then a table-specific `parse` constructor extracts the fixed fields and returns entry data as a byte slice for iteration. The `aml` submodule contains the bytecode `parser` (single-pass walker dispatching to an `AmlVisitor`), `path` (fixed-capacity namespace paths), `value` (resolved AML data objects), `visitor` (callback trait), and the optional `namespace` builder. The `resource` module provides a standalone resource descriptor parser for `_CRS` buffers. Shared infrastructure (`sdt` header, `rsdp` validation, `rsdt` enumeration) is factored into dedicated modules.
