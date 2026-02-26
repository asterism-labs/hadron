/* time.h — Time functions for Hadron libc */
#ifndef _TIME_H
#define _TIME_H

#include <stddef.h>
#include <sys/types.h>

typedef long time_t;

struct timespec {
    time_t tv_sec;
    long   tv_nsec;
};

#define CLOCK_REALTIME  0
#define CLOCK_MONOTONIC 1

int    clock_gettime(int clockid, struct timespec *tp);
int    nanosleep(const struct timespec *req, struct timespec *rem);
time_t time(time_t *tloc);

#endif /* _TIME_H */
