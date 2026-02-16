use limine::{
    BaseRevision, BootloaderInfoRequest, BootloaderPerformanceRequest, DateAtBootRequest,
    DeviceTreeBlobRequest, ExecutableAddressRequest, ExecutableCmdlineRequest, FirmwareTypeRequest,
    FramebufferRequest, HhdmRequest, MemMapRequest, ModuleRequest, PagingModeRequest,
    RequestsEndMarker, RequestsStartMarker, RsdpRequest, SmbiosRequest,
};

#[repr(C, align(8))]
pub struct LimineRequests {
    _start_marker: RequestsStartMarker,
    pub base_revision: BaseRevision,
    pub bootloader_info: BootloaderInfoRequest,
    pub firmware_type: FirmwareTypeRequest,
    pub memmap: MemMapRequest,
    pub hhdm: HhdmRequest,
    pub executable_address: ExecutableAddressRequest,
    pub framebuffer: FramebufferRequest,
    pub paging_mode: PagingModeRequest,
    pub rsdp: RsdpRequest,
    pub cmdline: ExecutableCmdlineRequest,
    pub dtb: DeviceTreeBlobRequest,
    pub smbios: SmbiosRequest,
    pub date_at_boot: DateAtBootRequest,
    pub modules: ModuleRequest,
    pub bootloader_performance: BootloaderPerformanceRequest,
    _end_marker: RequestsEndMarker,
}

impl LimineRequests {
    const fn new() -> Self {
        use limine::paging::PagingMode;
        Self {
            _start_marker: RequestsStartMarker::new(),
            base_revision: BaseRevision::new(),
            bootloader_info: BootloaderInfoRequest::new(),
            firmware_type: FirmwareTypeRequest::new(),
            memmap: MemMapRequest::new(),
            hhdm: HhdmRequest::new(),
            executable_address: ExecutableAddressRequest::new(),
            framebuffer: FramebufferRequest::new(),
            paging_mode: PagingModeRequest::new(
                PagingMode::Paging4Level,
                PagingMode::Paging4Level,
                PagingMode::Paging5Level,
            ),
            rsdp: RsdpRequest::new(),
            cmdline: ExecutableCmdlineRequest::new(),
            dtb: DeviceTreeBlobRequest::new(),
            smbios: SmbiosRequest::new(),
            date_at_boot: DateAtBootRequest::new(),
            modules: ModuleRequest::new(core::ptr::null(), 0),
            bootloader_performance: BootloaderPerformanceRequest::new(),
            _end_marker: RequestsEndMarker::new(),
        }
    }
}

// SAFETY: This struct is only read from by the bootloader, and is never mutated after
// initialization.
unsafe impl Sync for LimineRequests {}

#[used]
#[unsafe(link_section = ".requests")]
pub static REQUESTS: LimineRequests = LimineRequests::new();
