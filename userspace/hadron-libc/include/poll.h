/* poll.h — I/O multiplexing for Hadron libc */
#ifndef _POLL_H
#define _POLL_H

struct pollfd {
    int   fd;
    short events;
    short revents;
};

#define POLLIN   0x0001
#define POLLOUT  0x0004
#define POLLERR  0x0008
#define POLLHUP  0x0010
#define POLLNVAL 0x0020

typedef unsigned long nfds_t;

/* Implemented via sys_event_wait_many in hadron-libc-core */
int poll(struct pollfd *fds, nfds_t nfds, int timeout);

#endif /* _POLL_H */
