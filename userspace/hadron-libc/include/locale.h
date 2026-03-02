/* locale.h — Locale for Hadron libc */
#ifndef _LOCALE_H
#define _LOCALE_H

#include <bits/features.h>

#define LC_CTYPE    0
#define LC_NUMERIC  1
#define LC_TIME     2
#define LC_COLLATE  3
#define LC_MONETARY 4
#define LC_MESSAGES 5
#define LC_ALL      6

/* LC_*_MASK for newlocale */
#define LC_CTYPE_MASK    (1 << LC_CTYPE)
#define LC_NUMERIC_MASK  (1 << LC_NUMERIC)
#define LC_TIME_MASK     (1 << LC_TIME)
#define LC_COLLATE_MASK  (1 << LC_COLLATE)
#define LC_MONETARY_MASK (1 << LC_MONETARY)
#define LC_MESSAGES_MASK (1 << LC_MESSAGES)
#define LC_ALL_MASK      0x7f

struct lconv {
    char *decimal_point;
    char *thousands_sep;
    char *grouping;
    char *int_curr_symbol;
    char *currency_symbol;
    char *mon_decimal_point;
    char *mon_thousands_sep;
    char *mon_grouping;
    char *positive_sign;
    char *negative_sign;
};

/* locale_t: opaque locale object (POSIX.1-2008 / XSI) */
typedef void *locale_t;

#define LC_GLOBAL_LOCALE ((locale_t)-1)

#ifdef __cplusplus
extern "C" {
#endif

char         *setlocale(int category, const char *locale);
struct lconv *localeconv(void);

#if defined(_HADRON_POSIX_2008) || defined(_HADRON_DEFAULT)
locale_t newlocale(int category_mask, const char *locale, locale_t base);
locale_t duplocale(locale_t locobj);
void     freelocale(locale_t locobj);
locale_t uselocale(locale_t newloc);
#endif

#ifdef __cplusplus
}
#endif

#endif /* _LOCALE_H */
