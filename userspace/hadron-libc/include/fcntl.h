/* fcntl.h — File control for Hadron libc */
#ifndef _FCNTL_H
#define _FCNTL_H

#include <bits/features.h>

/* Open flags (POSIX values) */
#define O_RDONLY    0x0000
#define O_WRONLY    0x0001
#define O_RDWR      0x0002
#define O_CREAT     0x0040
#define O_EXCL      0x0080
#define O_TRUNC     0x0200
#define O_APPEND    0x0400
#define O_NONBLOCK  0x0800
#define O_DIRECTORY 0x10000
#define O_NOFOLLOW  0x20000
#define O_CLOEXEC   0x80000

/* fcntl commands */
#define F_DUPFD    0
#define F_GETFD    1
#define F_SETFD    2
#define F_GETFL    3
#define F_SETFL    4

/* File descriptor flags */
#define FD_CLOEXEC 1

int open(const char *path, int flags, ...);
int fcntl(int fd, int cmd, ...);

#endif /* _FCNTL_H */
