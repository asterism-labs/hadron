/* sys/select.h — Synchronous I/O multiplexing for Hadron libc (stub) */
#ifndef _SYS_SELECT_H
#define _SYS_SELECT_H

#include <bits/features.h>

#include <time.h>

#define FD_SETSIZE 1024

typedef struct {
    unsigned long fds_bits[FD_SETSIZE / (8 * sizeof(unsigned long))];
} fd_set;

#define FD_ZERO(set)   __builtin_memset((set), 0, sizeof(fd_set))
#define FD_SET(fd, set)   ((set)->fds_bits[(fd) / (8 * sizeof(unsigned long))] |= (1UL << ((fd) % (8 * sizeof(unsigned long)))))
#define FD_CLR(fd, set)   ((set)->fds_bits[(fd) / (8 * sizeof(unsigned long))] &= ~(1UL << ((fd) % (8 * sizeof(unsigned long)))))
#define FD_ISSET(fd, set) ((set)->fds_bits[(fd) / (8 * sizeof(unsigned long))] & (1UL << ((fd) % (8 * sizeof(unsigned long)))))

struct timeval {
    long tv_sec;
    long tv_usec;
};

int select(int nfds, fd_set *readfds, fd_set *writefds,
           fd_set *exceptfds, struct timeval *timeout);

#endif /* _SYS_SELECT_H */
