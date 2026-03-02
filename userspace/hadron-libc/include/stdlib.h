/* stdlib.h — General utilities for Hadron libc */
#ifndef _STDLIB_H
#define _STDLIB_H

#include <bits/features.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Memory allocation (C89) ----------------------------------------------- */

void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);
void  free(void *ptr);

/* ---- Process control (C89) ------------------------------------------------- */

void exit(int status) __attribute__((noreturn));
void _exit(int status) __attribute__((noreturn));
void _Exit(int status) __attribute__((noreturn));
void abort(void) __attribute__((noreturn));
int  atexit(void (*func)(void));

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1

/* ---- String → integer (C89) ------------------------------------------------ */

int          atoi(const char *s);
long         atol(const char *s);
long         strtol(const char *s, char **endptr, int base);
unsigned long strtoul(const char *s, char **endptr, int base);

/* ---- Integer arithmetic (C89) ---------------------------------------------- */

int  abs(int x);
long labs(long x);

typedef struct { int  quot; int  rem; } div_t;
typedef struct { long quot; long rem; } ldiv_t;

div_t  div(int numer, int denom);
ldiv_t ldiv(long numer, long denom);

/* ---- Search and sort (C89) ------------------------------------------------- */

void *bsearch(const void *key, const void *base, size_t nmemb, size_t size,
              int (*compar)(const void *, const void *));
void  qsort(void *base, size_t nmemb, size_t size,
            int (*compar)(const void *, const void *));

/* ---- Random number generation (C89) ---------------------------------------- */

#define RAND_MAX 2147483647
int  rand(void);
void srand(unsigned int seed);

/* ---- drand48 / lrand48 family (POSIX.1-2001 XSI extension) ---------------- */

#if defined(_HADRON_POSIX_2001) || defined(_HADRON_GNU_EXTENSIONS) || defined(_HADRON_DEFAULT)
double drand48(void);
long   lrand48(void);
long   mrand48(void);
void   srand48(long seedval);
long   nrand48(unsigned short xsubi[3]);
long   jrand48(unsigned short xsubi[3]);
void   lcong48(unsigned short param[7]);
unsigned short *seed48(unsigned short seed16v[3]);
#endif

/* ---- Multibyte / wide character (C89) -------------------------------------- */

int    mblen(const char *s, size_t n);
int    mbtowc(int *pwc, const char *s, size_t n);
int    wctomb(char *s, int wchar);
size_t mbstowcs(int *pwcs, const char *s, size_t n);
size_t wcstombs(char *s, const int *pwcs, size_t n);

/* ---- Environment ----------------------------------------------------------- */

char *getenv(const char *name);
int   setenv(const char *name, const char *value, int overwrite);
int   unsetenv(const char *name);
extern char **environ;

/* ---- C99 additions --------------------------------------------------------- */

long long           atoll(const char *s);
long long           strtoll(const char *s, char **endptr, int base);
unsigned long long  strtoull(const char *s, char **endptr, int base);
float               strtof(const char *s, char **endptr);
double              strtod(const char *s, char **endptr);
long double         strtold(const char *s, char **endptr);

long long  llabs(long long x);

typedef struct { long long quot; long long rem; } lldiv_t;
lldiv_t lldiv(long long numer, long long denom);

/* ---- C11 additions --------------------------------------------------------- */

#ifdef _HADRON_C11
void *aligned_alloc(size_t alignment, size_t size);
#endif

/* ---- POSIX.1-2001 additions ------------------------------------------------ */

#if defined(_HADRON_POSIX_2001) || defined(_HADRON_DEFAULT)
int posix_memalign(void **memptr, size_t alignment, size_t size);
long sysconf(int name);
int  getpagesize(void);
char *realpath(const char *path, char *resolved_path);
int  mkstemp(char *tmpl);
int  mkostemp(char *tmpl, int flags);
#endif

/* ---- POSIX.1-2008 additions ------------------------------------------------ */

#if defined(_HADRON_POSIX_2008) || defined(_HADRON_DEFAULT)
int mkstemps(char *tmpl, int suffixlen);
int mkostemps(char *tmpl, int suffixlen, int flags);
#endif

/* ---- GNU/BSD extensions ---------------------------------------------------- */

#if defined(_HADRON_GNU_EXTENSIONS) || defined(_HADRON_DEFAULT)
char *mktemp(char *tmpl);
#endif

#ifdef _HADRON_GNU_EXTENSIONS
int putenv(char *string);
void *memalign(size_t alignment, size_t size);
void *valloc(size_t size);
void *pvalloc(size_t size);
#endif

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* _STDLIB_H */
