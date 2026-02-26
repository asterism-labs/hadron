/* stdio.h — Standard I/O for Hadron libc */
#ifndef _STDIO_H
#define _STDIO_H

#include <stddef.h>
#include <stdint.h>

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

/* Formatted output */
int printf(const char *fmt, ...);
int fprintf(FILE *stream, const char *fmt, ...);
int sprintf(char *buf, const char *fmt, ...);
int snprintf(char *buf, size_t size, const char *fmt, ...);

/* Character/string I/O */
int   fputc(int c, FILE *stream);
int   fputs(const char *s, FILE *stream);
int   puts(const char *s);
int   putchar(int c);
char *fgets(char *s, int n, FILE *stream);

/* Binary I/O */
size_t fread(void *ptr, size_t size, size_t nmemb, FILE *stream);
size_t fwrite(const void *ptr, size_t size, size_t nmemb, FILE *stream);

/* File operations */
FILE *fopen(const char *path, const char *mode);
int   fclose(FILE *stream);
int   fflush(FILE *stream);

/* Positioning */
int  fseek(FILE *stream, long offset, int whence);
long ftell(FILE *stream);
void rewind(FILE *stream);

/* Status */
int feof(FILE *stream);
int ferror(FILE *stream);
void clearerr(FILE *stream);
int fileno(FILE *stream);

#endif /* _STDIO_H */
