/* strings.h — BSD/POSIX string functions for Hadron libc */
#ifndef _STRINGS_H
#define _STRINGS_H

#include <bits/features.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Always visible (BSD compat) ------------------------------------------- */

/* bzero and bcopy are obsolete; prefer memset/memmove */
#define bzero(s, n)        memset((s), 0, (n))
#define bcopy(src, dst, n) memmove((dst), (src), (n))

/* ---- POSIX.1-2001: case-insensitive comparison ----------------------------- */

#if defined(_HADRON_POSIX_2001) || defined(_HADRON_BSD) || defined(_HADRON_DEFAULT)
int strcasecmp(const char *s1, const char *s2);
int strncasecmp(const char *s1, const char *s2, size_t n);
#endif

/* ---- BSD: bit-scan functions ----------------------------------------------- */

#if defined(_HADRON_BSD) || defined(_HADRON_DEFAULT)
int ffs(int i);
int ffsl(long i);
int ffsll(long long i);
#endif

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* _STRINGS_H */
