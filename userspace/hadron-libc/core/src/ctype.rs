//! POSIX character classification functions.
//!
//! All functions operate on `int` (i32) values representing unsigned char values
//! or EOF (-1). Implemented via ASCII range checks.

#[unsafe(no_mangle)]
pub extern "C" fn isalpha(c: i32) -> i32 {
    i32::from(matches!(c as u8, b'A'..=b'Z' | b'a'..=b'z'))
}

#[unsafe(no_mangle)]
pub extern "C" fn isdigit(c: i32) -> i32 {
    i32::from(matches!(c as u8, b'0'..=b'9'))
}

#[unsafe(no_mangle)]
pub extern "C" fn isalnum(c: i32) -> i32 {
    i32::from(isalpha(c) != 0 || isdigit(c) != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn isspace(c: i32) -> i32 {
    i32::from(matches!(
        c as u8,
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn isupper(c: i32) -> i32 {
    i32::from(matches!(c as u8, b'A'..=b'Z'))
}

#[unsafe(no_mangle)]
pub extern "C" fn islower(c: i32) -> i32 {
    i32::from(matches!(c as u8, b'a'..=b'z'))
}

#[unsafe(no_mangle)]
pub extern "C" fn isprint(c: i32) -> i32 {
    i32::from(matches!(c as u8, 0x20..=0x7e))
}

#[unsafe(no_mangle)]
pub extern "C" fn iscntrl(c: i32) -> i32 {
    i32::from(matches!(c as u8, 0x00..=0x1f | 0x7f))
}

#[unsafe(no_mangle)]
pub extern "C" fn ispunct(c: i32) -> i32 {
    i32::from(isprint(c) != 0 && isalnum(c) == 0 && isspace(c) == 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn isxdigit(c: i32) -> i32 {
    i32::from(matches!(c as u8, b'0'..=b'9' | b'A'..=b'F' | b'a'..=b'f'))
}

#[unsafe(no_mangle)]
pub extern "C" fn isgraph(c: i32) -> i32 {
    i32::from(matches!(c as u8, 0x21..=0x7e))
}

#[unsafe(no_mangle)]
pub extern "C" fn toupper(c: i32) -> i32 {
    if islower(c) != 0 { c - 32 } else { c }
}

#[unsafe(no_mangle)]
pub extern "C" fn tolower(c: i32) -> i32 {
    if isupper(c) != 0 { c + 32 } else { c }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isalpha() {
        assert_ne!(isalpha(b'A' as i32), 0);
        assert_ne!(isalpha(b'z' as i32), 0);
        assert_eq!(isalpha(b'0' as i32), 0);
        assert_eq!(isalpha(b' ' as i32), 0);
    }

    #[test]
    fn test_isdigit() {
        assert_ne!(isdigit(b'0' as i32), 0);
        assert_ne!(isdigit(b'9' as i32), 0);
        assert_eq!(isdigit(b'a' as i32), 0);
    }

    #[test]
    fn test_isalnum() {
        assert_ne!(isalnum(b'A' as i32), 0);
        assert_ne!(isalnum(b'5' as i32), 0);
        assert_eq!(isalnum(b'!' as i32), 0);
    }

    #[test]
    fn test_isspace() {
        assert_ne!(isspace(b' ' as i32), 0);
        assert_ne!(isspace(b'\t' as i32), 0);
        assert_ne!(isspace(b'\n' as i32), 0);
        assert_eq!(isspace(b'a' as i32), 0);
    }

    #[test]
    fn test_isupper_islower() {
        assert_ne!(isupper(b'A' as i32), 0);
        assert_eq!(isupper(b'a' as i32), 0);
        assert_ne!(islower(b'a' as i32), 0);
        assert_eq!(islower(b'A' as i32), 0);
    }

    #[test]
    fn test_isprint() {
        assert_ne!(isprint(b' ' as i32), 0);
        assert_ne!(isprint(b'~' as i32), 0);
        assert_eq!(isprint(0x00), 0);
        assert_eq!(isprint(0x7f), 0);
    }

    #[test]
    fn test_iscntrl() {
        assert_ne!(iscntrl(0x00), 0);
        assert_ne!(iscntrl(0x1f), 0);
        assert_ne!(iscntrl(0x7f), 0);
        assert_eq!(iscntrl(b' ' as i32), 0);
    }

    #[test]
    fn test_ispunct() {
        assert_ne!(ispunct(b'!' as i32), 0);
        assert_ne!(ispunct(b'.' as i32), 0);
        assert_eq!(ispunct(b'a' as i32), 0);
        assert_eq!(ispunct(b' ' as i32), 0);
    }

    #[test]
    fn test_isxdigit() {
        assert_ne!(isxdigit(b'0' as i32), 0);
        assert_ne!(isxdigit(b'f' as i32), 0);
        assert_ne!(isxdigit(b'A' as i32), 0);
        assert_eq!(isxdigit(b'g' as i32), 0);
    }

    #[test]
    fn test_isgraph() {
        assert_ne!(isgraph(b'!' as i32), 0);
        assert_eq!(isgraph(b' ' as i32), 0);
    }

    #[test]
    fn test_toupper_tolower() {
        assert_eq!(toupper(b'a' as i32), b'A' as i32);
        assert_eq!(toupper(b'A' as i32), b'A' as i32);
        assert_eq!(toupper(b'5' as i32), b'5' as i32);
        assert_eq!(tolower(b'A' as i32), b'a' as i32);
        assert_eq!(tolower(b'a' as i32), b'a' as i32);
    }

    #[test]
    fn test_all_ascii() {
        for c in 0..=127i32 {
            // Every char should be exactly one of: cntrl, print
            let ctrl = iscntrl(c) != 0;
            let print = isprint(c) != 0;
            assert!(
                ctrl ^ print,
                "char {c} (0x{c:02x}): cntrl={ctrl}, print={print}"
            );
        }
    }
}
