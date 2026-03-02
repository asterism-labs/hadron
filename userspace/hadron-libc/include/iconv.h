/* iconv.h — Character set conversion for Hadron libc (POSIX.1-2001) */
#ifndef _ICONV_H
#define _ICONV_H

#include <bits/features.h>
#include <stddef.h>

typedef void *iconv_t;

#ifdef __cplusplus
extern "C" {
#endif

iconv_t iconv_open(const char *tocode, const char *fromcode);
size_t  iconv(iconv_t cd, char **inbuf, size_t *inbytesleft,
              char **outbuf, size_t *outbytesleft);
int     iconv_close(iconv_t cd);

#ifdef __cplusplus
}
#endif

#endif /* _ICONV_H */
