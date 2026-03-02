/* string.h — String and memory functions for Hadron libc */
#ifndef _STRING_H
#define _STRING_H

#include <bits/features.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Memory functions (C89) ----------------------------------------------- */

void *memcpy(void *dest, const void *src, size_t n);
void *memmove(void *dest, const void *src, size_t n);
void *memset(void *s, int c, size_t n);
int   memcmp(const void *s1, const void *s2, size_t n);
void *memchr(const void *s, int c, size_t n);

/* ---- String functions (C89) ----------------------------------------------- */

size_t strlen(const char *s);
int    strcmp(const char *s1, const char *s2);
int    strncmp(const char *s1, const char *s2, size_t n);
char  *strchr(const char *s, int c);
char  *strrchr(const char *s, int c);
char  *strstr(const char *haystack, const char *needle);
char  *strcpy(char *dest, const char *src);
char  *strncpy(char *dest, const char *src, size_t n);
char  *strcat(char *dest, const char *src);
char  *strncat(char *dest, const char *src, size_t n);
char  *strerror(int errnum);
size_t strxfrm(char *dest, const char *src, size_t n);
size_t strcspn(const char *s, const char *reject);
size_t strspn(const char *s, const char *accept);
char  *strpbrk(const char *s, const char *accept);
char  *strtok(char *str, const char *delim);

/* ---- C99 additions --------------------------------------------------------- */

#ifdef _HADRON_C99
/* strerror_r is technically POSIX, but also exposed with _HADRON_C99 */
#endif

/* ---- POSIX.1-2001 extensions ---------------------------------------------- */

#if defined(_HADRON_POSIX_2001) || defined(_HADRON_BSD) || defined(_HADRON_DEFAULT)
size_t strnlen(const char *s, size_t maxlen);
char  *strtok_r(char *str, const char *delim, char **saveptr);
int    strerror_r(int errnum, char *buf, size_t buflen);
#endif

/* ---- POSIX.1-2008 / BSD extensions ---------------------------------------- */

#if defined(_HADRON_POSIX_2008) || defined(_HADRON_BSD) || defined(_HADRON_DEFAULT)
char  *strdup(const char *s);
char  *strndup(const char *s, size_t n);
#endif

/* ---- BSD/POSIX: case-insensitive comparison -------------------------------- */

#if defined(_HADRON_BSD) || defined(_HADRON_POSIX_2001) || defined(_HADRON_DEFAULT)
int strcasecmp(const char *s1, const char *s2);
int strncasecmp(const char *s1, const char *s2, size_t n);
#endif

/* ---- GNU extensions -------------------------------------------------------- */

#ifdef _HADRON_GNU_EXTENSIONS
char  *strsignal(int sig);
char  *strchrnul(const char *s, int c);
void  *mempcpy(void *dest, const void *src, size_t n);
size_t strlcpy(char *dest, const char *src, size_t size);
size_t strlcat(char *dest, const char *src, size_t size);
#endif

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* _STRING_H */
