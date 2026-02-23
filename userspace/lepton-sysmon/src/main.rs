//! System dashboard — renders memory, process count, uptime, and kernel
//! version to the framebuffer in a 1-second refresh loop.
//!
//! Press `q` to quit. Requires `/dev/fb0` and stdin attached to a TTY.

#![no_std]
#![no_main]

use lepton_gfx::Surface;
use lepton_syslib::hadron_syscall::{ECHO, FBIOGET_INFO, FbInfo, ICANON, TCGETS, TCSETS, Termios};
use lepton_syslib::{io, println, sys};

// ── Colors ───────────────────────────────────────────────────────────

const BLACK: u32 = 0x0000_0000;
const WHITE: u32 = 0x00FF_FFFF;
const TITLE_BG: u32 = 0x0030_0060; // dark blue (BGR: R=0x60, G=0x00, B=0x30)
const TITLE_FG: u32 = 0x00FFFF00; // cyan (BGR: R=0x00, G=0xFF, B=0xFF)
const BAR_BG: u32 = 0x0040_4040; // dark grey
const BAR_FG: u32 = 0x0000_FF00; // green (BGR: R=0x00, G=0xFF, B=0x00)
const GREY: u32 = 0x0080_8080;

// ── Layout ───────────────────────────────────────────────────────────

const MARGIN_X: u32 = 20;
const TITLE_Y: u32 = 10;
const TITLE_H: u32 = 24;
const SECTION_Y: u32 = 50;
const LINE_H: u32 = 22;
const BAR_H: u32 = 16;

// ── No-alloc number formatting ───────────────────────────────────────

/// Format a `u64` into a stack buffer, returning the ASCII slice.
fn write_u64(buf: &mut [u8; 20], val: u64) -> &str {
    if val == 0 {
        buf[19] = b'0';
        // SAFETY: Single ASCII digit is valid UTF-8.
        return unsafe { core::str::from_utf8_unchecked(&buf[19..]) };
    }
    let mut pos = 20;
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    // SAFETY: All bytes written are ASCII digits, which are valid UTF-8.
    unsafe { core::str::from_utf8_unchecked(&buf[pos..]) }
}

/// Format a `u64` as zero-padded 2-digit decimal.
fn write_padded2(buf: &mut [u8; 2], val: u64) {
    buf[0] = b'0' + ((val / 10) % 10) as u8;
    buf[1] = b'0' + (val % 10) as u8;
}

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    // ── Open framebuffer ─────────────────────────────────────────────
    let fd = io::open("/dev/fb0", 0);
    if fd < 0 {
        println!("sysmon: failed to open /dev/fb0 (err={})", fd);
        return 1;
    }
    let fd = fd as usize;

    let mut fb_info = FbInfo {
        width: 0,
        height: 0,
        pitch: 0,
        bpp: 0,
        pixel_format: 0,
    };
    if io::ioctl(
        fd,
        FBIOGET_INFO as usize,
        &mut fb_info as *mut FbInfo as usize,
    ) < 0
    {
        println!("sysmon: ioctl FBIOGET_INFO failed");
        io::close(fd);
        return 1;
    }

    let fb_size = fb_info.pitch as usize * fb_info.height as usize;
    let fb_ptr = match sys::mem_map_device(fd, fb_size) {
        Some(p) => p,
        None => {
            println!("sysmon: mmap failed");
            io::close(fd);
            return 1;
        }
    };

    // ── Set raw mode on stdin ────────────────────────────────────────
    let mut orig_termios = Termios {
        iflag: 0,
        oflag: 0,
        cflag: 0,
        lflag: 0,
        cc: [0; 32],
    };
    io::ioctl(
        0,
        TCGETS as usize,
        &mut orig_termios as *mut Termios as usize,
    );

    let mut raw = orig_termios;
    raw.lflag &= !(ICANON | ECHO);
    io::ioctl(0, TCSETS as usize, &raw as *const Termios as usize);

    // ── Main loop ────────────────────────────────────────────────────
    let stride = fb_info.pitch / 4;
    let pixel_count = (stride * fb_info.height) as usize;

    loop {
        // Query system info.
        let mem = sys::query_memory();
        let procs = sys::query_processes();
        let uptime = sys::query_uptime();
        let version = sys::query_kernel_version();

        // Create surface and clear.
        // SAFETY: The kernel mapped `fb_size` bytes at `fb_ptr`. We interpret it
        // as u32 pixels (32bpp). The region is valid for this process's lifetime.
        let pixels = unsafe { core::slice::from_raw_parts_mut(fb_ptr as *mut u32, pixel_count) };
        let mut s = Surface::from_raw(pixels, fb_info.width, fb_info.height, stride);
        s.fill(BLACK);

        // ── Title bar ────────────────────────────────────────────────
        s.fill_rect(0, TITLE_Y, fb_info.width, TITLE_H, TITLE_BG);
        s.draw_str(
            MARGIN_X,
            TITLE_Y + 4,
            "Hadron System Dashboard",
            TITLE_FG,
            TITLE_BG,
        );

        let mut y = SECTION_Y;

        // ── Memory ───────────────────────────────────────────────────
        if let Some(mem) = mem {
            let used_mib = mem.used_bytes / (1024 * 1024);
            let total_mib = mem.total_bytes / (1024 * 1024);

            s.draw_str(MARGIN_X, y, "Memory:", WHITE, BLACK);
            y += LINE_H;

            // Bar background.
            let bar_w = fb_info.width - MARGIN_X * 2;
            s.fill_rect(MARGIN_X, y, bar_w, BAR_H, BAR_BG);

            // Bar fill (proportional).
            if total_mib > 0 {
                let fill_w = ((used_mib * bar_w as u64) / total_mib) as u32;
                s.fill_rect(MARGIN_X, y, fill_w, BAR_H, BAR_FG);
            }

            // Label: "NNN MiB / MMM MiB"
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
            // SAFETY: All bytes are ASCII, which is valid UTF-8.
            let label_str = unsafe { core::str::from_utf8_unchecked(&label[..pos]) };
            s.draw_str(MARGIN_X + 4, y, label_str, WHITE, BAR_BG);
            y += LINE_H + 4;
        }

        // ── Processes ────────────────────────────────────────────────
        if let Some(procs) = procs {
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
            // SAFETY: All bytes are ASCII.
            let label_str = unsafe { core::str::from_utf8_unchecked(&label[..pos]) };
            s.draw_str(MARGIN_X, y, label_str, WHITE, BLACK);
            y += LINE_H;
        }

        // ── Uptime ───────────────────────────────────────────────────
        if let Some(uptime) = uptime {
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

            // "Uptime: HH:MM:SS"
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
            // SAFETY: All bytes are ASCII.
            let label_str = unsafe { core::str::from_utf8_unchecked(&label[..pos]) };
            s.draw_str(MARGIN_X, y, label_str, WHITE, BLACK);
            y += LINE_H;
        }

        // ── Kernel version ───────────────────────────────────────────
        if let Some(ver) = version {
            let mut label = [0u8; 60];
            let mut pos = 0;
            for b in b"Kernel: " {
                if pos < label.len() {
                    label[pos] = *b;
                    pos += 1;
                }
            }
            // Name (NUL-terminated).
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
            // SAFETY: All bytes are ASCII.
            let label_str = unsafe { core::str::from_utf8_unchecked(&label[..pos]) };
            s.draw_str(MARGIN_X, y, label_str, WHITE, BLACK);
            y += LINE_H;
        }

        // ── Quit hint ────────────────────────────────────────────────
        let hint_y = fb_info.height.saturating_sub(30);
        s.draw_str(
            MARGIN_X,
            hint_y.max(y + LINE_H),
            "Press 'q' to quit",
            GREY,
            BLACK,
        );

        // ── Poll stdin for 'q' ───────────────────────────────────────
        if sys::poll_stdin() {
            let mut buf = [0u8; 1];
            let n = io::read(0, &mut buf);
            if n > 0 && buf[0] == b'q' {
                break;
            }
        }

        sys::sleep_secs(1);
    }

    // ── Clear screen and restore terminal ───────────────────────────
    {
        // SAFETY: fb_ptr is still mapped; clear framebuffer before exit.
        let pixels = unsafe { core::slice::from_raw_parts_mut(fb_ptr as *mut u32, pixel_count) };
        let mut s = Surface::from_raw(pixels, fb_info.width, fb_info.height, stride);
        s.fill(BLACK);
    }
    io::ioctl(0, TCSETS as usize, &orig_termios as *const Termios as usize);
    sys::mem_unmap(fb_ptr, fb_size);
    io::close(fd);
    0
}
