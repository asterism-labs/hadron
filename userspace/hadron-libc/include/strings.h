/* strings.h — BSD string functions for Hadron libc */
#ifndef _STRINGS_H
#define _STRINGS_H

#include <stddef.h>

/* These are provided as macros wrapping standard functions. */
#define bzero(s, n)       memset((s), 0, (n))
#define bcopy(src, dst, n) memmove((dst), (src), (n))

#endif /* _STRINGS_H */
