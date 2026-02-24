//! System dashboard — renders memory, process count, uptime, and kernel
//! version to a compositor surface in a 1-second refresh loop.
//!
//! Press `q` to quit. When running under the compositor, receives keyboard
//! events via the display protocol. Falls back to direct framebuffer access
//! when no compositor is available (fd 3 not present).

#![no_std]
#![no_main]

use lepton_display_client::{Display, Event};
use lepton_gfx::Surface;
use lepton_syslib::{println, sys};

// ── Colors ───────────────────────────────────────────────────────────

const BLACK: u32 = 0x0000_0000;
const WHITE: u32 = 0x00FF_FFFF;
const TITLE_BG: u32 = 0x0030_0060;
const TITLE_FG: u32 = 0x00FF_FF00;
const BAR_BG: u32 = 0x0040_4040;
const BAR_FG: u32 = 0x0000_FF00;
const GREY: u32 = 0x0080_8080;

// ── Layout ───────────────────────────────────────────────────────────

const MARGIN_X: u32 = 10;
const TITLE_Y: u32 = 4;
const TITLE_H: u32 = 20;
const SECTION_Y: u32 = 30;
const LINE_H: u32 = 18;
const BAR_H: u32 = 14;

// ── No-alloc number formatting ───────────────────────────────────────

/// Format a `u64` into a stack buffer, returning the ASCII slice.
fn write_u64(buf: &mut [u8; 20], val: u64) -> &str {
    if val == 0 {
        buf[19] = b'0';
        return core::str::from_utf8(&buf[19..]).expect("ASCII digit");
    }
    let mut pos = 20;
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    core::str::from_utf8(&buf[pos..]).expect("ASCII digits")
}

/// Format a `u64` as zero-padded 2-digit decimal.
fn write_padded2(buf: &mut [u8; 2], val: u64) {
    buf[0] = b'0' + ((val / 10) % 10) as u8;
    buf[1] = b'0' + (val % 10) as u8;
}

/// Render the dashboard onto a surface.
fn render(s: &mut Surface<'_>) {
    let width = s.width();

    s.fill(BLACK);

    // ── Title bar ────────────────────────────────────────────────────
    s.fill_rect(0, TITLE_Y, width, TITLE_H, TITLE_BG);
    s.draw_str(
        MARGIN_X,
        TITLE_Y + 2,
        "Hadron System Dashboard",
        TITLE_FG,
        TITLE_BG,
    );

    let mut y = SECTION_Y;

    // ── Memory ───────────────────────────────────────────────────────
    if let Some(mem) = sys::query_memory() {
        let used_mib = mem.used_bytes / (1024 * 1024);
        let total_mib = mem.total_bytes / (1024 * 1024);

        s.draw_str(MARGIN_X, y, "Memory:", WHITE, BLACK);
        y += LINE_H;

        let bar_w = width - MARGIN_X * 2;
        s.fill_rect(MARGIN_X, y, bar_w, BAR_H, BAR_BG);

        if total_mib > 0 {
            let fill_w = ((used_mib * bar_w as u64) / total_mib) as u32;
            s.fill_rect(MARGIN_X, y, fill_w, BAR_H, BAR_FG);
        }

        let mut nbuf = [0u8; 20];
        let used_s = write_u64(&mut nbuf, used_mib);
        let mut label = [0u8; 40];
        let mut pos = 0;
        for b in used_s.bytes() {
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        for b in b" MiB / " {
            if pos < label.len() {
                label[pos] = *b;
                pos += 1;
            }
        }
        let mut nbuf2 = [0u8; 20];
        let total_s = write_u64(&mut nbuf2, total_mib);
        for b in total_s.bytes() {
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        for b in b" MiB" {
            if pos < label.len() {
                label[pos] = *b;
                pos += 1;
            }
        }
        let label_str = core::str::from_utf8(&label[..pos]).expect("ASCII label");
        s.draw_str(MARGIN_X + 4, y, label_str, WHITE, BAR_BG);
        y += LINE_H + 4;
    }

    // ── Processes ────────────────────────────────────────────────────
    if let Some(procs) = sys::query_processes() {
        let mut nbuf = [0u8; 20];
        let count_s = write_u64(&mut nbuf, u64::from(procs.count));
        let mut label = [0u8; 40];
        let mut pos = 0;
        for b in b"Processes: " {
            if pos < label.len() {
                label[pos] = *b;
                pos += 1;
            }
        }
        for b in count_s.bytes() {
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        let label_str = core::str::from_utf8(&label[..pos]).expect("ASCII label");
        s.draw_str(MARGIN_X, y, label_str, WHITE, BLACK);
        y += LINE_H;
    }

    // ── Uptime ───────────────────────────────────────────────────────
    if let Some(uptime) = sys::query_uptime() {
        let total_secs = uptime.uptime_ns / 1_000_000_000;
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;

        let mut hbuf = [0u8; 20];
        let hours_s = write_u64(&mut hbuf, hours);
        let mut mbuf = [0u8; 2];
        write_padded2(&mut mbuf, mins);
        let mut sbuf = [0u8; 2];
        write_padded2(&mut sbuf, secs);

        let mut label = [0u8; 40];
        let mut pos = 0;
        for b in b"Uptime: " {
            if pos < label.len() {
                label[pos] = *b;
                pos += 1;
            }
        }
        for b in hours_s.bytes() {
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        if pos < label.len() {
            label[pos] = b':';
            pos += 1;
        }
        for b in &mbuf {
            if pos < label.len() {
                label[pos] = *b;
                pos += 1;
            }
        }
        if pos < label.len() {
            label[pos] = b':';
            pos += 1;
        }
        for b in &sbuf {
            if pos < label.len() {
                label[pos] = *b;
                pos += 1;
            }
        }
        let label_str = core::str::from_utf8(&label[..pos]).expect("ASCII label");
        s.draw_str(MARGIN_X, y, label_str, WHITE, BLACK);
        y += LINE_H;
    }

    // ── Kernel version ───────────────────────────────────────────────
    if let Some(ver) = sys::query_kernel_version() {
        let mut label = [0u8; 60];
        let mut pos = 0;
        for b in b"Kernel: " {
            if pos < label.len() {
                label[pos] = *b;
                pos += 1;
            }
        }
        for &b in &ver.name {
            if b == 0 {
                break;
            }
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        if pos < label.len() {
            label[pos] = b' ';
            pos += 1;
        }
        let mut nbuf = [0u8; 20];
        let major_s = write_u64(&mut nbuf, u64::from(ver.major));
        for b in major_s.bytes() {
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        if pos < label.len() {
            label[pos] = b'.';
            pos += 1;
        }
        let mut nbuf2 = [0u8; 20];
        let minor_s = write_u64(&mut nbuf2, u64::from(ver.minor));
        for b in minor_s.bytes() {
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        if pos < label.len() {
            label[pos] = b'.';
            pos += 1;
        }
        let mut nbuf3 = [0u8; 20];
        let patch_s = write_u64(&mut nbuf3, u64::from(ver.patch));
        for b in patch_s.bytes() {
            if pos < label.len() {
                label[pos] = b;
                pos += 1;
            }
        }
        let label_str = core::str::from_utf8(&label[..pos]).expect("ASCII label");
        s.draw_str(MARGIN_X, y, label_str, WHITE, BLACK);
        y += LINE_H;
    }

    // ── Quit hint ────────────────────────────────────────────────────
    let height = s.height();
    let hint_y = height.saturating_sub(20);
    s.draw_str(
        MARGIN_X,
        hint_y.max(y + LINE_H),
        "Press 'q' to quit",
        GREY,
        BLACK,
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    let Some(mut display) = Display::connect() else {
        println!("sysmon: failed to connect to compositor");
        return 1;
    };

    loop {
        let mut surface = display.surface();
        render(&mut surface);
        drop(surface);
        display.commit();

        // Poll for events.
        while let Some(event) = display.poll_event() {
            if let Event::Key {
                character: b'q', ..
            } = event
            {
                display.disconnect();
                return 0;
            }
        }

        sys::sleep_secs(1);
    }
}
