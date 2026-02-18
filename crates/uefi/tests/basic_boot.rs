#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![allow(missing_docs)]

use core::fmt::Write;

use uefi::api::{Boot, Gop, SystemTable};
use uefi::api::{GraphicsOutputId, SimpleFileSystemId};
use uefi::protocol::simple_text::color;

hadron_test::uefi_test_entry_point!();

/// Helper: construct a safe `SystemTable<Boot>` from the test harness globals.
///
/// # Safety
/// Each test function creates its own wrapper. This is sound for sequential
/// test execution in a single-threaded UEFI environment as long as we don't
/// call `exit_boot_services`.
unsafe fn boot_table() -> SystemTable<Boot> {
    let handle = unsafe { image_handle() };
    let raw_st = SYSTEM_TABLE.load(core::sync::atomic::Ordering::Acquire);
    unsafe { SystemTable::from_raw(handle, raw_st) }
}

// ── SystemTable tests ─────────────────────────────────────────────────

#[test_case]
fn system_table_image_handle_is_non_null() {
    let st = unsafe { boot_table() };
    assert!(!st.image_handle().is_null());
}

// ── BootServices tests ────────────────────────────────────────────────

#[test_case]
fn boot_services_stall() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();
    assert!(bs.stall(1).is_ok());
}

#[test_case]
fn boot_services_set_watchdog_timer() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();
    // Disable the watchdog timer (timeout = 0)
    assert!(bs.set_watchdog_timer(0, 0).is_ok());
}

// ── Console tests ─────────────────────────────────────────────────────

#[test_case]
fn console_out_is_usable() {
    let st = unsafe { boot_table() };
    let con = st.console_out();
    assert!(con.output_string("Hello from safe API\n").is_ok());
}

#[test_case]
fn console_out_clear_screen() {
    let st = unsafe { boot_table() };
    let con = st.console_out();
    assert!(con.clear_screen().is_ok());

    let mode = con.mode();
    assert_eq!(mode.cursor_column, 0);
    assert_eq!(mode.cursor_row, 0);
}

#[test_case]
fn console_out_set_cursor_position() {
    let st = unsafe { boot_table() };
    let con = st.console_out();
    con.clear_screen().unwrap();
    assert!(con.set_cursor_position(5, 3).is_ok());

    let mode = con.mode();
    assert_eq!(mode.cursor_column, 5);
    assert_eq!(mode.cursor_row, 3);
}

#[test_case]
fn console_out_set_attribute() {
    let st = unsafe { boot_table() };
    let con = st.console_out();
    assert!(con.set_attribute(color::YELLOW, color::BLUE).is_ok());

    let expected = color::attribute(color::YELLOW, color::BLUE);
    assert_eq!(con.mode().attribute as usize, expected);
}

#[test_case]
fn console_out_reset() {
    let st = unsafe { boot_table() };
    let con = st.console_out();
    assert!(con.reset().is_ok());

    let mode = con.mode();
    assert_eq!(mode.cursor_column, 0);
    assert_eq!(mode.cursor_row, 0);
}

#[test_case]
fn console_out_mode_is_valid() {
    let st = unsafe { boot_table() };
    let con = st.console_out();
    assert!(con.mode().max_mode >= 1);
}

#[test_case]
fn console_out_fmt_write() {
    let st = unsafe { boot_table() };
    let mut con = st.console_out();
    assert!(write!(con, "formatted: {} + {} = {}\n", 1, 2, 3).is_ok());
}

#[test_case]
fn console_err_is_usable() {
    let st = unsafe { boot_table() };
    let con = st.console_err();
    assert!(con.output_string("stderr works\n").is_ok());
}

// ── MemoryMap tests ───────────────────────────────────────────────────

#[test_case]
fn memory_map_is_non_empty() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();

    let mut buf = [0u8; 8192];
    let map = bs.get_memory_map(&mut buf).expect("get_memory_map failed");

    assert!(!map.is_empty());
    assert!(map.descriptor_size() >= core::mem::size_of::<uefi::memory::EfiMemoryDescriptor>());
}

#[test_case]
fn memory_map_iteration() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();

    let mut buf = [0u8; 8192];
    let map = bs.get_memory_map(&mut buf).expect("get_memory_map failed");

    let mut count = 0;
    for desc in &map {
        // Each descriptor should have a valid memory type (0..=15)
        assert!(
            desc.memory_type <= 15,
            "unexpected memory type: {}",
            desc.memory_type
        );
        count += 1;
    }
    assert_eq!(count, map.len());
    assert!(count > 0);
}

// ── GOP tests ─────────────────────────────────────────────────────────

#[test_case]
fn gop_current_mode() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();

    if let Ok(raw) = bs.locate_protocol::<GraphicsOutputId>() {
        let gop = Gop::new(raw);
        let info = gop.current_mode();
        assert!(info.horizontal_resolution > 0);
        assert!(info.vertical_resolution > 0);
    }
    // GOP may not be present in headless QEMU — not a failure
}

#[test_case]
fn gop_framebuffer() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();

    if let Ok(raw) = bs.locate_protocol::<GraphicsOutputId>() {
        let gop = Gop::new(raw);
        assert!(gop.frame_buffer_base() != 0);
        assert!(gop.frame_buffer_size() > 0);
    }
}

#[test_case]
fn gop_query_mode() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();

    if let Ok(raw) = bs.locate_protocol::<GraphicsOutputId>() {
        let gop = Gop::new(raw);
        assert!(gop.max_mode() >= 1);

        let info = gop.query_mode(0).expect("query_mode(0) failed");
        assert!(info.horizontal_resolution > 0);
        assert!(info.vertical_resolution > 0);
    }
}

// ── FileSystem tests ──────────────────────────────────────────────────

#[test_case]
fn filesystem_open_volume() {
    let st = unsafe { boot_table() };
    let bs = st.boot_services();

    if let Ok(raw) = bs.locate_protocol::<SimpleFileSystemId>() {
        let fs = uefi::api::FileSystem::new(raw);
        let _root = fs.open_volume().expect("open_volume failed");
        // File is closed on drop — success if we get here
    }
}
