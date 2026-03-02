/* errno.h — Error numbers for Hadron libc */
#ifndef _ERRNO_H
#define _ERRNO_H

#include <bits/features.h>

/* errno is accessed via __errno_location() for future thread-safety. */
int *__errno_location(void);
#define errno (*__errno_location())

#define EPERM           1
#define ENOENT          2
#define ESRCH           3
#define EINTR           4
#define EIO             5
#define ENXIO           6
#define E2BIG           7
#define ENOEXEC         8
#define EBADF           9
#define ECHILD         10
#define EAGAIN         11
#define ENOMEM         12
#define EACCES         13
#define EFAULT         14
#define EBUSY          16
#define EEXIST         17
#define EXDEV          18
#define ENODEV         19
#define ENOTDIR        20
#define EISDIR         21
#define EINVAL         22
#define ENFILE         23
#define EMFILE         24
#define ENOTTY         25
#define EFBIG          27
#define ENOSPC         28
#define ESPIPE         29
#define EROFS          30
#define EPIPE          32
#define ERANGE         34
#define ENOSYS         38
#define ENOTEMPTY      39
#define ELOOP          40
#define EWOULDBLOCK    EAGAIN
#define ETIMEDOUT     110
#define EADDRNOTAVAIL 99
#define EADDRINUSE    98
#define ECONNREFUSED 111
#define ECONNRESET   104
#define ECONNABORTED 103
#define ENETDOWN     100
#define ENETUNREACH  101
#define EHOSTUNREACH 113
#define ENOTSUP       95
#define EOPNOTSUPP    95
#define EOVERFLOW     75
#define ENOBUFS      105
#define ENOLINK      126
#define EMSGSIZE      90
#define EPROTOTYPE    91
#define ENOPROTOOPT   92
#define EPROTONOSUPPORT 93
#define ESOCKTNOSUPPORT 94
#define EALREADY     114
#define EINPROGRESS  115
#define EDESTADDRREQ  89
#define EAFNOSUPPORT  97
#define EILSEQ        84
#define ENOMSG        42
#define EIDRM         43
#define EDEADLK       35
#define ENOLCK        37
#define ECANCELED    125
#define EOWNERDEAD   130
#define ENOTRECOVERABLE 131

#endif /* _ERRNO_H */
