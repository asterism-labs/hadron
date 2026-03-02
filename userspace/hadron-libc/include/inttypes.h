/* inttypes.h — printf/scanf format macros for fixed-width integer types */
#ifndef _INTTYPES_H
#define _INTTYPES_H

#include <bits/features.h>

#include <stdint.h>

/* ── printf format macros ──────────────────────────────────────────────── */

#define PRId8   "hhd"
#define PRId16  "hd"
#define PRId32  "d"
#define PRId64  "ld"

#define PRIi8   "hhi"
#define PRIi16  "hi"
#define PRIi32  "i"
#define PRIi64  "li"

#define PRIo8   "hho"
#define PRIo16  "ho"
#define PRIo32  "o"
#define PRIo64  "lo"

#define PRIu8   "hhu"
#define PRIu16  "hu"
#define PRIu32  "u"
#define PRIu64  "lu"

#define PRIx8   "hhx"
#define PRIx16  "hx"
#define PRIx32  "x"
#define PRIx64  "lx"

#define PRIX8   "hhX"
#define PRIX16  "hX"
#define PRIX32  "X"
#define PRIX64  "lX"

#define PRIdPTR "ld"
#define PRIiPTR "li"
#define PRIoPTR "lo"
#define PRIuPTR "lu"
#define PRIxPTR "lx"
#define PRIXPTR "lX"

/* ── scanf format macros ───────────────────────────────────────────────── */

#define SCNd8   "hhd"
#define SCNd16  "hd"
#define SCNd32  "d"
#define SCNd64  "ld"

#define SCNi8   "hhi"
#define SCNi16  "hi"
#define SCNi32  "i"
#define SCNi64  "li"

#define SCNo8   "hho"
#define SCNo16  "ho"
#define SCNo32  "o"
#define SCNo64  "lo"

#define SCNu8   "hhu"
#define SCNu16  "hu"
#define SCNu32  "u"
#define SCNu64  "lu"

#define SCNx8   "hhx"
#define SCNx16  "hx"
#define SCNx32  "x"
#define SCNx64  "lx"

#define SCNdPTR "ld"
#define SCNiPTR "li"
#define SCNoPTR "lo"
#define SCNuPTR "lu"
#define SCNxPTR "lx"

/* ── imaxdiv_t ─────────────────────────────────────────────────────────── */

typedef struct {
    intmax_t quot;
    intmax_t rem;
} imaxdiv_t;

/* ── functions ─────────────────────────────────────────────────────────── */

intmax_t  imaxabs(intmax_t j);
imaxdiv_t imaxdiv(intmax_t numer, intmax_t denom);
intmax_t  strtoimax(const char *nptr, char **endptr, int base);
uintmax_t strtoumax(const char *nptr, char **endptr, int base);

#endif /* _INTTYPES_H */
