/* libgen.h — POSIX basename and dirname */
#ifndef _LIBGEN_H
#define _LIBGEN_H

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Return the last component of path (may modify path in place).
 * Thread-unsafe; use the GNU basename(__) extension when available.
 */
char *basename(char *path);

/**
 * Return the directory component of path (may modify path in place).
 */
char *dirname(char *path);

#ifdef __cplusplus
}
#endif

#endif /* _LIBGEN_H */
