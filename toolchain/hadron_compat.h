/*
 * hadron_compat.h — Force-included compatibility header for Mesa cross-compilation.
 *
 * This header is injected via -include before any Mesa source file, allowing
 * compiler-assisted patching: we pre-define OS detection macros so that Mesa's
 * #ifndef guards in detect_os.h don't override them to 0.
 *
 * Usage: clang -include toolchain/hadron_compat.h ...
 */
#ifndef HADRON_COMPAT_H
#define HADRON_COMPAT_H

/* --- OS detection overrides ------------------------------------------------
 *
 * Mesa's src/util/detect_os.h checks for __hadron__ (which we define via
 * -D__hadron__=1 in the cross file), but the detection block and fallback
 * defaults don't exist upstream. By pre-defining these macros here, the
 * #ifndef DETECT_OS_HADRON / #define DETECT_OS_HADRON 0 fallback in
 * detect_os.h is bypassed.
 */
#define DETECT_OS_HADRON 1
#define DETECT_OS_POSIX  1

/* --- Feature availability --------------------------------------------------
 *
 * Tell Mesa which POSIX features Hadron's libc provides, so that Meson's
 * has_function() probes and #if HAVE_* checks resolve correctly even when
 * cross-compiling (where runtime probes are impossible).
 */

/* sysconf(_SC_PHYS_PAGES) and sysconf(_SC_PAGESIZE) are implemented. */
#ifndef HAVE_SYSCONF
#define HAVE_SYSCONF 1
#endif

/* We provide pthread stubs (real implementation in Phase B). */
#ifndef HAVE_PTHREAD
#define HAVE_PTHREAD 1
#endif

/* No dynamic linking on Hadron. */
#ifndef HAVE_DLOPEN
#define HAVE_DLOPEN 0
#endif

/* No shared memory or DRM. */
#ifndef HAVE_SHM
#define HAVE_SHM 0
#endif

/* No Linux-specific interfaces. */
#ifndef HAVE_LINUX_FUTEX_H
#define HAVE_LINUX_FUTEX_H 0
#endif

/* Hadron doesn't provide program_invocation_name yet. */
#ifndef HAVE_PROGRAM_INVOCATION_NAME
#define HAVE_PROGRAM_INVOCATION_NAME 0
#endif

/* We provide timespec_get via time.h stubs. */
#ifndef HAVE_TIMESPEC_GET
#define HAVE_TIMESPEC_GET 0
#endif

/* strtod/strtof are provided (minimal implementation). */
#ifndef HAVE_STRTOD
#define HAVE_STRTOD 1
#endif

/* No libdrm. */
#ifndef HAVE_LIBDRM
#define HAVE_LIBDRM 0
#endif

/* --- Missing type/macro polyfills ------------------------------------------
 *
 * Some Mesa code assumes Linux-specific macros. Provide fallbacks here
 * rather than patching each callsite.
 */

/* Mesa's ralloc uses this for debug naming. */
#ifndef __FILE_NAME__
#define __FILE_NAME__ __FILE__
#endif

#endif /* HADRON_COMPAT_H */
