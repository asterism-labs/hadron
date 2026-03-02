/* endian.h — byte-order definitions (x86-64 is always little-endian) */
#ifndef _ENDIAN_H
#define _ENDIAN_H

#include <bits/features.h>

#define __LITTLE_ENDIAN 1234
#define __BIG_ENDIAN    4321
#define __PDP_ENDIAN    3412

#define __BYTE_ORDER    __LITTLE_ENDIAN
#define __FLOAT_WORD_ORDER __LITTLE_ENDIAN

/* POSIX names */
#define LITTLE_ENDIAN   __LITTLE_ENDIAN
#define BIG_ENDIAN      __BIG_ENDIAN
#define PDP_ENDIAN      __PDP_ENDIAN
#define BYTE_ORDER      __BYTE_ORDER

/* Conversion macros — x86-64: le == host, be == swap */

static inline unsigned short __bswap16(unsigned short x) {
    return (unsigned short)((x >> 8) | (x << 8));
}
static inline unsigned int __bswap32(unsigned int x) {
    return ((x >> 24) | ((x >> 8) & 0xff00u) | ((x << 8) & 0xff0000u) | (x << 24));
}
static inline unsigned long long __bswap64(unsigned long long x) {
    return (((unsigned long long)__bswap32((unsigned int)x) << 32) |
            (unsigned long long)__bswap32((unsigned int)(x >> 32)));
}

#define htobe16(x)  __bswap16(x)
#define htole16(x)  (x)
#define be16toh(x)  __bswap16(x)
#define le16toh(x)  (x)

#define htobe32(x)  __bswap32(x)
#define htole32(x)  (x)
#define be32toh(x)  __bswap32(x)
#define le32toh(x)  (x)

#define htobe64(x)  __bswap64(x)
#define htole64(x)  (x)
#define be64toh(x)  __bswap64(x)
#define le64toh(x)  (x)

#endif /* _ENDIAN_H */
