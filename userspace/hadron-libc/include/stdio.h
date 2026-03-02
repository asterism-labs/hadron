/* stdio.h — Standard I/O for Hadron libc */
#ifndef _STDIO_H
#define _STDIO_H

#include <bits/features.h>
#include <stddef.h>
#include <stdarg.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque FILE type — actual layout is in Rust. */
typedef struct _FILE FILE;

/* Standard streams accessed via functions. */
FILE *__stdin(void);
FILE *__stdout(void);
FILE *__stderr(void);

#define stdin  (__stdin())
#define stdout (__stdout())
#define stderr (__stderr())

#define EOF    (-1)
#define BUFSIZ 4096

/* Buffering modes for setvbuf */
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

/* Seek origins */
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

/* ---- C89 formatted output -------------------------------------------------- */

int printf(const char *fmt, ...);
int fprintf(FILE *stream, const char *fmt, ...);
int sprintf(char *buf, const char *fmt, ...);
int vprintf(const char *fmt, va_list ap);
int vfprintf(FILE *stream, const char *fmt, va_list ap);
int vsprintf(char *buf, const char *fmt, va_list ap);

/* ---- C99 additions --------------------------------------------------------- */

int snprintf(char *buf, size_t size, const char *fmt, ...);
int vsnprintf(char *buf, size_t size, const char *fmt, va_list ap);

/* ---- Formatted input ------------------------------------------------------- */

int scanf(const char *fmt, ...);
int fscanf(FILE *stream, const char *fmt, ...);
int sscanf(const char *str, const char *fmt, ...);
int vscanf(const char *fmt, va_list ap);
int vfscanf(FILE *stream, const char *fmt, va_list ap);
int vsscanf(const char *str, const char *fmt, va_list ap);

/* ---- Character/string I/O -------------------------------------------------- */

int   fputc(int c, FILE *stream);
int   fputs(const char *s, FILE *stream);
int   puts(const char *s);
int   putchar(int c);
int   fgetc(FILE *stream);
int   getc(FILE *stream);
int   getchar(void);
int   ungetc(int c, FILE *stream);
char *fgets(char *s, int n, FILE *stream);

/* ---- Binary I/O ------------------------------------------------------------ */

size_t fread(void *ptr, size_t size, size_t nmemb, FILE *stream);
size_t fwrite(const void *ptr, size_t size, size_t nmemb, FILE *stream);

/* ---- File operations ------------------------------------------------------- */

FILE *fopen(const char *path, const char *mode);
FILE *freopen(const char *path, const char *mode, FILE *stream);
int   fclose(FILE *stream);
int   fflush(FILE *stream);
void  setbuf(FILE *stream, char *buf);
int   setvbuf(FILE *stream, char *buf, int mode, size_t size);

/* ---- Positioning ----------------------------------------------------------- */

int  fseek(FILE *stream, long offset, int whence);
long ftell(FILE *stream);
void rewind(FILE *stream);

/* ---- Status ---------------------------------------------------------------- */

int  feof(FILE *stream);
int  ferror(FILE *stream);
void clearerr(FILE *stream);
int  fileno(FILE *stream);

/* ---- Error reporting ------------------------------------------------------- */

void perror(const char *s);

/* ---- POSIX.1-2001 additions ------------------------------------------------ */

#if defined(_HADRON_POSIX_2001) || defined(_HADRON_DEFAULT)
FILE *fdopen(int fd, const char *mode);
int   fseeko(FILE *stream, long long offset, int whence);
long long ftello(FILE *stream);
#endif

/* ---- GNU/POSIX.1-2008 additions ------------------------------------------- */

#if defined(_HADRON_POSIX_2008) || defined(_HADRON_GNU_EXTENSIONS) || defined(_HADRON_DEFAULT)
#include <sys/types.h>
ssize_t getline(char **lineptr, size_t *n, FILE *stream);
ssize_t getdelim(char **lineptr, size_t *n, int delim, FILE *stream);
#endif

/* ---- GNU extensions -------------------------------------------------------- */

#ifdef _HADRON_GNU_EXTENSIONS
int asprintf(char **strp, const char *fmt, ...);
int vasprintf(char **strp, const char *fmt, va_list ap);
#endif

/* ---- Temporary files ------------------------------------------------------- */

FILE *tmpfile(void);
char *tmpnam(char *s);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* _STDIO_H */
