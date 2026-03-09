//! CMOS Real-Time Clock (RTC) reader.
//!
//! Reads the date and time from the standard x86 CMOS RTC registers
//! via I/O ports 0x70 (index) and 0x71 (data). The RTC provides
//! wall-clock time in BCD or binary format depending on register B.
//!
//! This module only reads the RTC at boot to establish a Unix epoch
//! offset. It does not set up periodic RTC interrupts.
//!
//! # References
//!
//! - Motorola MC146818 RTC Datasheet
//!   <https://pdf1.alldatasheet.com/datasheet-pdf/view/122156/MOTOROLA/MC146818.html>
//! - OSDev Wiki: CMOS
//!   <https://wiki.osdev.org/CMOS>

use crate::arch::x86_64::instructions::port::Port;

/// CMOS index port.
const CMOS_INDEX: u16 = 0x70;
/// CMOS data port.
const CMOS_DATA: u16 = 0x71;

/// RTC register indices.
const RTC_SECONDS: u8 = 0x00;
const RTC_MINUTES: u8 = 0x02;
const RTC_HOURS: u8 = 0x04;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_CENTURY: u8 = 0x32; // Common default; ACPI FADT may override.
const RTC_STATUS_A: u8 = 0x0A;
const RTC_STATUS_B: u8 = 0x0B;

/// Read a CMOS register.
///
/// # Safety
///
/// Caller must ensure this is called with interrupts disabled or in
/// a context where NMI masking is acceptable.
unsafe fn cmos_read(reg: u8) -> u8 {
    let index_port = Port::<u8>::new(CMOS_INDEX);
    let data_port = Port::<u8>::new(CMOS_DATA);
    // Bit 7 of index port = NMI disable. We set it to avoid NMI
    // during the read sequence.
    unsafe {
        index_port.write(0x80 | reg);
        data_port.read()
    }
}

/// Convert BCD byte to binary.
const fn bcd_to_bin(bcd: u8) -> u8 {
    (bcd & 0x0F) + (bcd >> 4) * 10
}

/// Date/time snapshot from the CMOS RTC.
#[derive(Debug, Clone, Copy)]
struct RtcTime {
    seconds: u8,
    minutes: u8,
    hours: u8,
    day: u8,
    month: u8,
    year: u16,
}

/// Wait until the RTC update-in-progress flag clears.
///
/// # Safety
///
/// Same as `cmos_read`.
unsafe fn wait_for_rtc_ready() {
    // Spin while bit 7 of Status Register A is set (update in progress).
    while unsafe { cmos_read(RTC_STATUS_A) } & 0x80 != 0 {
        core::hint::spin_loop();
    }
}

/// Read the current date/time from the CMOS RTC.
///
/// Uses double-read to avoid tearing during an RTC update cycle.
///
/// # Safety
///
/// Must be called early in boot when interrupts are disabled.
pub unsafe fn read_rtc() -> u64 {
    // Read twice to ensure consistency (no mid-update tearing).
    let time = unsafe {
        let mut prev;
        let mut curr = read_rtc_raw();
        loop {
            prev = curr;
            wait_for_rtc_ready();
            curr = read_rtc_raw();
            if prev.seconds == curr.seconds
                && prev.minutes == curr.minutes
                && prev.hours == curr.hours
                && prev.day == curr.day
                && prev.month == curr.month
                && prev.year == curr.year
            {
                break;
            }
        }
        curr
    };

    // Convert to Unix timestamp (seconds since 1970-01-01 00:00:00 UTC).
    datetime_to_unix(
        time.year as u64,
        time.month as u64,
        time.day as u64,
        time.hours as u64,
        time.minutes as u64,
        time.seconds as u64,
    )
}

/// Read raw RTC registers and convert from BCD if needed.
///
/// # Safety
///
/// Same as `cmos_read`.
unsafe fn read_rtc_raw() -> RtcTime {
    unsafe {
        wait_for_rtc_ready();
    }

    let raw_sec = unsafe { cmos_read(RTC_SECONDS) };
    let raw_min = unsafe { cmos_read(RTC_MINUTES) };
    let raw_hour = unsafe { cmos_read(RTC_HOURS) };
    let raw_day = unsafe { cmos_read(RTC_DAY) };
    let raw_month = unsafe { cmos_read(RTC_MONTH) };
    let raw_year = unsafe { cmos_read(RTC_YEAR) };
    let raw_century = unsafe { cmos_read(RTC_CENTURY) };
    let status_b = unsafe { cmos_read(RTC_STATUS_B) };

    let is_bcd = status_b & 0x04 == 0;

    let (seconds, minutes, hours, day, month, year_low, century) = if is_bcd {
        (
            bcd_to_bin(raw_sec),
            bcd_to_bin(raw_min),
            bcd_to_bin(raw_hour & 0x7F), // Mask 12h/24h bit
            bcd_to_bin(raw_day),
            bcd_to_bin(raw_month),
            bcd_to_bin(raw_year),
            bcd_to_bin(raw_century),
        )
    } else {
        (
            raw_sec,
            raw_min,
            raw_hour & 0x7F,
            raw_day,
            raw_month,
            raw_year,
            raw_century,
        )
    };

    let year = if century > 0 {
        century as u16 * 100 + year_low as u16
    } else {
        // Assume 2000s if century register unavailable.
        2000 + year_low as u16
    };

    // Handle 12-hour mode: if bit 1 of status_b is clear, we're in 12h mode.
    let hours = if status_b & 0x02 == 0 && raw_hour & 0x80 != 0 {
        // PM flag set in 12-hour mode.
        (hours % 12) + 12
    } else {
        hours
    };

    RtcTime {
        seconds,
        minutes,
        hours,
        day,
        month,
        year,
    }
}

/// Convert a date/time to Unix timestamp (seconds since epoch).
///
/// Uses a simplified algorithm valid for years >= 1970.
fn datetime_to_unix(year: u64, month: u64, day: u64, hour: u64, min: u64, sec: u64) -> u64 {
    // Days from 1970-01-01 to the start of `year`.
    let mut days = 0u64;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }

    // Days from Jan 1 to the start of `month`.
    const MONTH_DAYS: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += MONTH_DAYS[(m - 1) as usize];
        if m == 2 && is_leap_year(year) {
            days += 1;
        }
    }

    days += day - 1; // Days are 1-indexed.
    days * 86400 + hour * 3600 + min * 60 + sec
}

/// Returns `true` if `year` is a leap year.
const fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
