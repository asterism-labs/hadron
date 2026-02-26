//! TTY line discipline — cooked-mode line editing and scancode processing.
//!
//! This crate provides the pure-logic portion of the TTY subsystem: scancode
//! decoding, modifier tracking, line editing, and the ready buffer. It is
//! host-testable (no kernel dependencies).

#![no_std]
#![warn(missing_docs)]

pub mod ldisc;

pub use ldisc::{LdiscAction, LineDiscipline};

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::vec::Vec;

    /// Helper: feed a string as ASCII bytes in canonical mode and return collected actions.
    fn feed_ascii(ld: &mut LineDiscipline, bytes: &[u8]) -> Vec<LdiscAction> {
        bytes
            .iter()
            .filter_map(|&b| ld.process_ascii_byte(b, true, true))
            .collect()
    }

    /// Helper: read all available bytes from the line discipline.
    fn read_all(ld: &mut LineDiscipline) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        while let Some(n) = ld.try_read(&mut buf) {
            if n == 0 {
                break; // EOF
            }
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    #[test]
    fn cooked_mode_line_editing() {
        let mut ld = LineDiscipline::new();
        // Type "hello\n"
        let actions = feed_ascii(&mut ld, b"hello\n");
        // Last action should be Newline
        assert!(matches!(actions.last(), Some(LdiscAction::Newline)));
        // Read should produce "hello\n"
        let data = read_all(&mut ld);
        assert_eq!(data, b"hello\n");
    }

    #[test]
    fn backspace_erases_character() {
        let mut ld = LineDiscipline::new();
        // Type "abc", backspace, then "d\n"
        feed_ascii(&mut ld, b"abc");
        let actions = feed_ascii(&mut ld, &[0x7F]); // DEL = backspace
        assert!(matches!(actions[0], LdiscAction::Backspace));
        feed_ascii(&mut ld, b"d\n");
        let data = read_all(&mut ld);
        assert_eq!(data, b"abd\n");
    }

    #[test]
    fn backspace_on_empty_line_no_action() {
        let mut ld = LineDiscipline::new();
        let actions = feed_ascii(&mut ld, &[0x08]); // BS on empty line
        assert!(actions.is_empty());
    }

    #[test]
    fn ctrl_c_generates_interrupt() {
        let mut ld = LineDiscipline::new();
        feed_ascii(&mut ld, b"partial");
        let actions: Vec<_> = [0x03u8]
            .iter()
            .filter_map(|&b| ld.process_ascii_byte(b, true, true))
            .collect();
        assert!(matches!(actions[0], LdiscAction::Interrupt));
        // Line buffer should be cleared
        feed_ascii(&mut ld, b"\n");
        let data = read_all(&mut ld);
        assert_eq!(data, b"\n"); // only newline, no "partial"
    }

    #[test]
    fn ctrl_c_raw_signal_delivers_literal() {
        let mut ld = LineDiscipline::new();
        // isig = false: Ctrl+C delivered as literal 0x03
        let actions: Vec<_> = [0x03u8]
            .iter()
            .filter_map(|&b| ld.process_ascii_byte(b, true, false))
            .collect();
        assert!(matches!(actions[0], LdiscAction::Char(0x03)));
        let data = read_all(&mut ld);
        assert_eq!(data, &[0x03]);
    }

    #[test]
    fn ctrl_d_eof_on_empty_line() {
        let mut ld = LineDiscipline::new();
        let actions: Vec<_> = [0x04u8]
            .iter()
            .filter_map(|&b| ld.process_ascii_byte(b, true, true))
            .collect();
        assert!(matches!(actions[0], LdiscAction::Eof));
        // try_read should return Some(0) for EOF
        let mut buf = [0u8; 16];
        assert_eq!(ld.try_read(&mut buf), Some(0));
    }

    #[test]
    fn ctrl_d_flushes_partial_line() {
        let mut ld = LineDiscipline::new();
        feed_ascii(&mut ld, b"hi");
        let actions: Vec<_> = [0x04u8]
            .iter()
            .filter_map(|&b| ld.process_ascii_byte(b, true, true))
            .collect();
        assert!(matches!(actions[0], LdiscAction::FlushLine));
        let data = read_all(&mut ld);
        assert_eq!(data, b"hi"); // no trailing newline
    }

    #[test]
    fn raw_mode_passthrough() {
        let mut ld = LineDiscipline::new();
        // icanon = false: bytes go directly to ready buffer
        for &b in b"raw" {
            ld.process_ascii_byte(b, false, true);
        }
        let data = read_all(&mut ld);
        assert_eq!(data, b"raw");
    }

    #[test]
    fn has_data_reflects_state() {
        let mut ld = LineDiscipline::new();
        assert!(!ld.has_data());
        feed_ascii(&mut ld, b"x\n");
        assert!(ld.has_data());
        read_all(&mut ld);
        assert!(!ld.has_data());
    }

    #[test]
    fn cr_converts_to_newline_in_canonical() {
        let mut ld = LineDiscipline::new();
        feed_ascii(&mut ld, b"hi\r");
        let data = read_all(&mut ld);
        assert_eq!(data, b"hi\n"); // CR → newline in canonical
    }

    #[test]
    fn try_read_returns_none_when_empty() {
        let mut ld = LineDiscipline::new();
        let mut buf = [0u8; 16];
        assert_eq!(ld.try_read(&mut buf), None);
    }
}
