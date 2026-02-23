//! Framebuffer demo — draws a gradient and colored rectangles to `/dev/fb0`.
//!
//! Opens the framebuffer device, queries its dimensions via ioctl, mmaps the
//! pixel buffer, and draws a static image using `lepton-gfx`.

#![no_std]
#![no_main]

use lepton_gfx::{Surface, bgr};
use lepton_syslib::hadron_syscall::{FBIOGET_INFO, FbInfo};
use lepton_syslib::{io, println, sys};

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    // 1. Open /dev/fb0
    let fd = io::open("/dev/fb0", 0);
    if fd < 0 {
        println!("fbdemo: failed to open /dev/fb0 (err={})", fd);
        return 1;
    }
    let fd = fd as usize;

    // 2. Query framebuffer info
    let mut info = FbInfo {
        width: 0,
        height: 0,
        pitch: 0,
        bpp: 0,
        pixel_format: 0,
    };
    let ret = io::ioctl(fd, FBIOGET_INFO as usize, &mut info as *mut FbInfo as usize);
    if ret < 0 {
        println!("fbdemo: ioctl FBIOGET_INFO failed (err={})", ret);
        io::close(fd);
        return 1;
    }

    println!(
        "fbdemo: {}x{} pitch={} bpp={} fmt={}",
        info.width, info.height, info.pitch, info.bpp, info.pixel_format
    );

    // 3. mmap the framebuffer
    let size = info.pitch as usize * info.height as usize;
    let ptr = match sys::mem_map_device(fd, size) {
        Some(p) => p,
        None => {
            println!("fbdemo: mmap failed");
            io::close(fd);
            return 1;
        }
    };

    // 4. Create Surface and draw
    let stride = info.pitch / 4; // pixels per row for 32bpp
    let pixel_count = (stride * info.height) as usize;
    // SAFETY: The kernel mapped `size` bytes of framebuffer memory at `ptr`.
    // We interpret it as u32 pixels (32bpp). The region is valid for the
    // lifetime of this process.
    let pixels = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u32, pixel_count) };
    let mut surface = Surface::from_raw(pixels, info.width, info.height, stride);

    // Draw a gradient background (purple-ish: red varies by Y, blue by X)
    for y in 0..info.height {
        let r = ((y as u64 * 255) / info.height as u64) as u8;
        for x in 0..info.width {
            let b = ((x as u64 * 255) / info.width as u64) as u8;
            surface.put_pixel(x, y, bgr(r, 0, b));
        }
    }

    // Draw colored rectangles
    surface.fill_rect(100, 100, 200, 150, bgr(255, 0, 0)); // red
    surface.fill_rect(350, 200, 200, 150, bgr(0, 255, 0)); // green
    surface.fill_rect(600, 300, 200, 150, bgr(0, 0, 255)); // blue

    // Draw a white border rectangle (outline only)
    let white = bgr(255, 255, 255);
    surface.hline(50, 50, 400, white);
    surface.hline(50, 450, 400, white);
    surface.vline(50, 50, 400, white);
    surface.vline(449, 50, 401, white);

    println!("fbdemo: drawing complete");

    // 5. Clean up
    sys::mem_unmap(ptr, size);
    io::close(fd);
    0
}
