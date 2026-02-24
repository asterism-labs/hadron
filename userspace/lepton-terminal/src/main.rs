//! GUI terminal emulator for the Hadron compositor.
//!
//! Opens a PTY master/slave pair, spawns `/bin/sh` on the slave side, and
//! renders the shell's output to a compositor surface. Keyboard events from
//! the compositor are forwarded to the PTY master.

#![no_std]
#![no_main]

extern crate alloc;

mod ansi;
mod grid;
mod render;

use lepton_display_client::{Display, Event};
use lepton_gfx::font;
use lepton_syslib::hadron_syscall::{TCSETS, TIOCGPTN, TIOCSPTLCK, TIOCSWINSZ, Termios, Winsize};
use lepton_syslib::{io, println, sys};

use crate::grid::Grid;

/// Target frame interval in milliseconds (~30 fps).
const FRAME_MS: u64 = 33;

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    // 1. Connect to the compositor.
    let mut display = match Display::connect() {
        Some(d) => d,
        None => {
            println!("terminal: failed to connect to compositor");
            return 1;
        }
    };

    let width = display.width();
    let height = display.height();
    let cols = (width / font::WIDTH) as usize;
    let rows = (height / font::HEIGHT) as usize;

    if cols == 0 || rows == 0 {
        println!("terminal: surface too small for terminal");
        return 1;
    }

    // 2. Open PTY master (/dev/ptmx).
    let master_fd = io::open("/dev/ptmx", 0);
    if master_fd < 0 {
        println!("terminal: failed to open /dev/ptmx");
        return 1;
    }
    let master_fd = master_fd as usize;

    // Get the slave PTY index.
    let mut pty_num: u32 = 0;
    if io::ioctl(
        master_fd,
        TIOCGPTN as usize,
        &mut pty_num as *mut u32 as usize,
    ) < 0
    {
        println!("terminal: TIOCGPTN failed");
        return 1;
    }

    // Unlock the slave.
    let lock_val: u32 = 0;
    io::ioctl(
        master_fd,
        TIOCSPTLCK as usize,
        &lock_val as *const u32 as usize,
    );

    // 3. Configure the master: raw mode (no ICANON, no ECHO).
    let raw_termios = Termios {
        iflag: 0,
        oflag: 0,
        cflag: 0,
        lflag: 0,
        cc: [0; 32],
    };
    io::ioctl(
        master_fd,
        TCSETS as usize,
        &raw_termios as *const Termios as usize,
    );

    // 4. Set window size.
    let winsize = Winsize {
        rows: rows as u16,
        cols: cols as u16,
        xpixel: width as u16,
        ypixel: height as u16,
    };
    io::ioctl(
        master_fd,
        TIOCSWINSZ as usize,
        &winsize as *const Winsize as usize,
    );

    // 5. Open the slave side.
    let mut pts_path = [0u8; 32];
    let prefix = b"/dev/pts/";
    pts_path[..prefix.len()].copy_from_slice(prefix);
    let num_len = write_u32(&mut pts_path[prefix.len()..], pty_num);
    let path_len = prefix.len() + num_len;
    let pts_str = core::str::from_utf8(&pts_path[..path_len]).unwrap_or("/dev/pts/0");

    let slave_fd = io::open(pts_str, 0);
    if slave_fd < 0 {
        println!("terminal: failed to open {}", pts_str);
        return 1;
    }
    let slave_fd = slave_fd as usize;

    // 6. Spawn the shell with the slave as stdin/stdout/stderr.
    let pid = sys::spawn_with_fds(
        "/bin/sh",
        &["/bin/sh"],
        &[
            (0, slave_fd as u32),
            (1, slave_fd as u32),
            (2, slave_fd as u32),
        ],
    );

    // Close the slave fd — the shell owns it now.
    io::close(slave_fd);

    if pid < 0 {
        println!("terminal: failed to spawn shell: {}", pid);
        return 1;
    }

    // 7. Main loop.
    let mut grid = Grid::new(cols, rows);
    let mut read_buf = [0u8; 512];

    loop {
        // Read shell output from the PTY master.
        if sys::poll_fd_read(master_fd) {
            let n = io::read(master_fd, &mut read_buf);
            if n > 0 {
                grid.feed_bytes(&read_buf[..n as usize]);
            } else if n == 0 {
                // Shell exited (EOF on master).
                break;
            }
        }

        // Poll compositor events (keyboard input).
        while let Some(event) = display.poll_event() {
            match event {
                Event::Key {
                    character, pressed, ..
                } => {
                    if pressed && character != 0 {
                        io::write(master_fd, &[character]);
                    }
                }
                Event::FocusGained | Event::FocusLost => {}
                Event::Mouse { .. } => {}
            }
        }

        // Render if dirty.
        if grid.dirty {
            let mut surface = display.surface();
            render::render_grid(&mut surface, &grid);
            display.commit();
            grid.dirty = false;
        }

        sys::sleep_ms(FRAME_MS);
    }

    io::close(master_fd);
    display.disconnect();
    0
}

/// Write a u32 as decimal ASCII into a buffer, returning the number of bytes written.
fn write_u32(buf: &mut [u8], val: u32) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
        }
        return 1;
    }
    // Write digits in reverse order, then we already have forward order
    // because we write most-significant first.
    let mut tmp = [0u8; 10];
    let mut pos = 0;
    let mut v = val;
    while v > 0 {
        tmp[pos] = b'0' + (v % 10) as u8;
        v /= 10;
        pos += 1;
    }
    // Reverse into buf.
    let len = pos;
    for i in 0..len {
        if i < buf.len() {
            buf[i] = tmp[len - 1 - i];
        }
    }
    len
}
