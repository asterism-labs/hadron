/* bits/features.h — feature-test macro normalization for Hadron libc.
 *
 * This header must be included first by every public libc header.
 * It translates POSIX/GNU/BSD feature-test macros into internal
 * _HADRON_* macros that guard symbol visibility.
 *
 * Standard macros recognized (checked before including any libc header):
 *
 *   _GNU_SOURCE          — GNU extensions + all POSIX + all XSI + BSD
 *   _XOPEN_SOURCE <N>    — X/Open (SUS) level; 600 = SUSv3, 700 = SUSv4/POSIX.1-2008
 *   _POSIX_C_SOURCE <N>  — POSIX level; 200809L = POSIX.1-2008, 200112L = POSIX.1-2001
 *   _POSIX_SOURCE        — deprecated alias for POSIX.1-1990
 *   _BSD_SOURCE          — BSD 4.3 extensions (deprecated in glibc >= 2.20; use _DEFAULT_SOURCE)
 *   _DEFAULT_SOURCE      — default symbol set (like _BSD_SOURCE, replaces implicit defaults)
 *   __STRICT_ANSI__      — only ANSI/ISO C symbols (set by -ansi, -std=c89, -std=c99, etc.)
 *
 * Internal macros set by this header (never define these yourself):
 *
 *   _HADRON_GNU_EXTENSIONS  — GNU-specific extensions (__attribute__ etc.)
 *   _HADRON_POSIX_2008      — POSIX.1-2008 (Issue 7) symbols visible
 *   _HADRON_POSIX_2001      — POSIX.1-2001 (Issue 6) symbols visible
 *   _HADRON_POSIX_1990      — POSIX.1-1990 (Issue 1) symbols visible
 *   _HADRON_XOPEN           — X/Open extensions visible
 *   _HADRON_BSD             — BSD extensions visible (strdup, strlcpy, etc.)
 *   _HADRON_DEFAULT         — default visibility (implicit when no standard specified)
 */

#ifndef _HADRON_BITS_FEATURES_H
#define _HADRON_BITS_FEATURES_H

/* ---- _GNU_SOURCE implies everything ---------------------------------------- */

#ifdef _GNU_SOURCE
#  undef  _POSIX_SOURCE
#  define _POSIX_SOURCE 1
#  undef  _POSIX_C_SOURCE
#  define _POSIX_C_SOURCE 200809L
#  undef  _XOPEN_SOURCE
#  define _XOPEN_SOURCE 700
#  define _XOPEN_SOURCE_EXTENDED 1
#  undef  _BSD_SOURCE
#  define _BSD_SOURCE 1
#  undef  _DEFAULT_SOURCE
#  define _DEFAULT_SOURCE 1
#  define _HADRON_GNU_EXTENSIONS 1
#endif

/* ---- Implicit default when no standard is pinned --------------------------- */

#if !defined(__STRICT_ANSI__)          \
 && !defined(_POSIX_SOURCE)            \
 && !defined(_POSIX_C_SOURCE)          \
 && !defined(_XOPEN_SOURCE)            \
 && !defined(_GNU_SOURCE)              \
 && !defined(_BSD_SOURCE)              \
 && !defined(_DEFAULT_SOURCE)
#  define _DEFAULT_SOURCE 1
#endif

/* ---- _DEFAULT_SOURCE implies BSD + POSIX.1-2008 --------------------------- */

#ifdef _DEFAULT_SOURCE
#  define _HADRON_DEFAULT 1
#  define _HADRON_BSD 1
#  ifndef _POSIX_C_SOURCE
#    define _POSIX_C_SOURCE 200809L
#  endif
#endif

/* ---- _BSD_SOURCE ----------------------------------------------------------- */

#ifdef _BSD_SOURCE
#  define _HADRON_BSD 1
#endif

/* ---- _XOPEN_SOURCE --------------------------------------------------------- */

#if defined(_XOPEN_SOURCE) && _XOPEN_SOURCE >= 500
#  define _HADRON_XOPEN 1
#endif

/* ---- _POSIX_SOURCE (legacy alias for POSIX.1-1990) ------------------------- */

#ifdef _POSIX_SOURCE
#  ifndef _POSIX_C_SOURCE
#    define _POSIX_C_SOURCE 1L
#  endif
#endif

/* ---- Derive POSIX level from _POSIX_C_SOURCE ------------------------------- */

#if defined(_POSIX_C_SOURCE)
#  if _POSIX_C_SOURCE >= 1L
#    define _HADRON_POSIX_1990 1
#  endif
#  if _POSIX_C_SOURCE >= 199506L
#    define _HADRON_POSIX_1995 1
#  endif
#  if _POSIX_C_SOURCE >= 200112L
#    define _HADRON_POSIX_2001 1
#  endif
#  if _POSIX_C_SOURCE >= 200809L
#    define _HADRON_POSIX_2008 1
#  endif
#endif

/* ---- C standard version macros -------------------------------------------- */

#if defined(__STDC_VERSION__)
#  if __STDC_VERSION__ >= 201710L
#    define _HADRON_C17 1
#  endif
#  if __STDC_VERSION__ >= 201112L
#    define _HADRON_C11 1
#  endif
#  if __STDC_VERSION__ >= 199901L
#    define _HADRON_C99 1
#  endif
#endif

#endif /* _HADRON_BITS_FEATURES_H */
