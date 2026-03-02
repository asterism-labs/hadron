/* resolv.h — DNS resolver for Hadron libc (stub) */
#ifndef _RESOLV_H
#define _RESOLV_H

#include <bits/features.h>
#include <stddef.h>
#include <stdint.h>

/* Minimal resolver state (opaque) */
struct __res_state {
    int  retrans;
    int  retry;
    unsigned long options;
    int  nscount;
    int  id;
    unsigned char _pad[128];
};
typedef struct __res_state *res_state;

extern struct __res_state _res;

/* dn_expand: expand a compressed domain name */
int dn_expand(const unsigned char *msg, const unsigned char *eom,
              const unsigned char *src, char *dst, int dstsiz);

/* dn_comp: compress a domain name */
int dn_comp(const char *src, unsigned char *dst, int dstsiz,
            unsigned char **dnptrs, unsigned char **lastdnptr);

/* res_init, res_query */
int res_init(void);
int res_query(const char *dname, int cls, int type,
              unsigned char *answer, int anslen);

#endif /* _RESOLV_H */
