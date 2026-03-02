/* sys/time.h — Time types for Hadron libc */
#ifndef _SYS_TIME_H
#define _SYS_TIME_H

#include <bits/features.h>

#include <time.h>

struct timeval {
    long tv_sec;
    long tv_usec;
};

struct timezone {
    int tz_minuteswest;
    int tz_dsttime;
};

int gettimeofday(struct timeval *tv, struct timezone *tz);

#endif /* _SYS_TIME_H */
