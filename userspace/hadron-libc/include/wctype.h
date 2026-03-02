/* wctype.h — Wide character classification for Hadron libc (C99) */
#ifndef _WCTYPE_H
#define _WCTYPE_H

#include <bits/features.h>
#include <wchar.h>

typedef unsigned long wctype_t;
typedef unsigned long wctrans_t;

#ifdef __cplusplus
extern "C" {
#endif

/* Classification functions */
int iswalpha(wint_t wc);
int iswdigit(wint_t wc);
int iswalnum(wint_t wc);
int iswspace(wint_t wc);
int iswupper(wint_t wc);
int iswlower(wint_t wc);
int iswprint(wint_t wc);
int iswpunct(wint_t wc);
int iswcntrl(wint_t wc);
int iswblank(wint_t wc);
int iswgraph(wint_t wc);
int iswxdigit(wint_t wc);

/* Case conversion */
wint_t towupper(wint_t wc);
wint_t towlower(wint_t wc);

/* Generic classification */
wctype_t  wctype(const char *property);
int       iswctype(wint_t wc, wctype_t desc);
wctrans_t wctrans(const char *property);
wint_t    towctrans(wint_t wc, wctrans_t desc);

#ifdef __cplusplus
}
#endif

#endif /* _WCTYPE_H */
