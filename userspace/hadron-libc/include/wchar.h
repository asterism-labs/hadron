/* wchar.h — wide character type and minimal stubs for Hadron */
#ifndef _WCHAR_H
#define _WCHAR_H

#include <bits/features.h>

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#ifndef WCHAR_MIN
#define WCHAR_MIN (-2147483647 - 1)
#endif
#ifndef WCHAR_MAX
#define WCHAR_MAX 2147483647
#endif
#ifndef WEOF
#define WEOF     ((wint_t)-1)
#endif

typedef unsigned int wint_t;
typedef struct { int __state; } mbstate_t;

/* Wide string functions (minimal stubs; Mesa rarely uses them) */
size_t  wcslen(const wchar_t *s);
wchar_t *wcscpy(wchar_t *dst, const wchar_t *src);
wchar_t *wcsncpy(wchar_t *dst, const wchar_t *src, size_t n);
int     wcscmp(const wchar_t *s1, const wchar_t *s2);
int     wcsncmp(const wchar_t *s1, const wchar_t *s2, size_t n);
wchar_t *wcscat(wchar_t *dst, const wchar_t *src);
wchar_t *wcschr(const wchar_t *s, wchar_t c);
size_t  wcstombs(char *dst, const wchar_t *src, size_t n);
size_t  mbstowcs(wchar_t *dst, const char *src, size_t n);
int     wctomb(char *s, wchar_t wc);
int     mbtowc(wchar_t *pwc, const char *s, size_t n);
size_t  mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps);
size_t  mbsrtowcs(wchar_t *dst, const char **src, size_t len, mbstate_t *ps);
size_t  wcrtomb(char *s, wchar_t wc, mbstate_t *ps);
wchar_t *wcsstr(const wchar_t *haystack, const wchar_t *needle);
wchar_t *wcsncat(wchar_t *dst, const wchar_t *src, size_t n);
int      wcscoll(const wchar_t *s1, const wchar_t *s2);
wchar_t *wcspbrk(const wchar_t *s, const wchar_t *accept);
size_t   wcsspn(const wchar_t *s, const wchar_t *accept);
size_t   wcscspn(const wchar_t *s, const wchar_t *reject);
wchar_t *wcstok(wchar_t *s, const wchar_t *delim, wchar_t **ptr);
int      swprintf(wchar_t *wcs, size_t maxlen, const wchar_t *fmt, ...);
int      swscanf(const wchar_t *wcs, const wchar_t *fmt, ...);

#ifdef __cplusplus
}
#endif

#endif /* _WCHAR_H */
