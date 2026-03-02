/* arpa/inet.h — Internet address conversion for Hadron libc */
#ifndef _ARPA_INET_H
#define _ARPA_INET_H

#include <bits/features.h>
#include <stdint.h>
#include <sys/types.h>

/* Address family for inet_pton / inet_ntop */
#ifndef AF_INET
# define AF_INET   2
# define AF_INET6 10
#endif

typedef uint32_t in_addr_t;
typedef uint16_t in_port_t;

struct in_addr {
    in_addr_t s_addr;
};

/* Byte order conversion */
static inline uint32_t __bswap32(uint32_t x) {
    return ((x & 0xff000000u) >> 24) | ((x & 0x00ff0000u) >> 8) |
           ((x & 0x0000ff00u) << 8)  | ((x & 0x000000ffu) << 24);
}
static inline uint16_t __bswap16(uint16_t x) {
    return (uint16_t)(((x & 0xff00u) >> 8) | ((x & 0x00ffu) << 8));
}

#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
# define htonl(x) __bswap32(x)
# define htons(x) __bswap16(x)
# define ntohl(x) __bswap32(x)
# define ntohs(x) __bswap16(x)
#else
# define htonl(x) (x)
# define htons(x) (x)
# define ntohl(x) (x)
# define ntohs(x) (x)
#endif

#ifndef INET_ADDRSTRLEN
# define INET_ADDRSTRLEN  16
# define INET6_ADDRSTRLEN 46
#endif

#ifdef __cplusplus
extern "C" {
#endif

const char *inet_ntop(int af, const void *src, char *dst, unsigned int size);
int         inet_pton(int af, const char *src, void *dst);
in_addr_t   inet_addr(const char *cp);
char       *inet_ntoa(struct in_addr in);

#ifdef __cplusplus
}
#endif

#endif /* _ARPA_INET_H */
