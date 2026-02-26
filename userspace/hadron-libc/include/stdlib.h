/* stdlib.h — General utilities for Hadron libc */
#ifndef _STDLIB_H
#define _STDLIB_H

#include <stddef.h>

/* Memory allocation */
void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);
void  free(void *ptr);

/* Process control */
void   exit(int status) __attribute__((noreturn));
void  _exit(int status) __attribute__((noreturn));
void   abort(void) __attribute__((noreturn));
int    atexit(void (*func)(void));

/* String conversion */
int  atoi(const char *s);
long atol(const char *s);
long          strtol(const char *s, char **endptr, int base);
unsigned long strtoul(const char *s, char **endptr, int base);

/* Integer arithmetic */
int  abs(int x);
long labs(long x);

/* Environment */
char *getenv(const char *name);
int   setenv(const char *name, const char *value, int overwrite);
int   unsetenv(const char *name);
extern char **environ;

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1

#endif /* _STDLIB_H */
