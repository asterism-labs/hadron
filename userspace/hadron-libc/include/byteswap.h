/* byteswap.h — glibc-compatible bswap_* macros */
#ifndef _BYTESWAP_H
#define _BYTESWAP_H

#include <bits/features.h>

#include <endian.h>

#define bswap_16(x)  __bswap16(x)
#define bswap_32(x)  __bswap32(x)
#define bswap_64(x)  __bswap64(x)

#endif /* _BYTESWAP_H */
