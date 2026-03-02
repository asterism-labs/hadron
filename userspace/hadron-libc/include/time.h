/* time.h — Time functions for Hadron libc */
#ifndef _TIME_H
#define _TIME_H

#include <bits/features.h>

#include <stddef.h>
#include <sys/types.h>

typedef long time_t;
typedef int  clockid_t;

struct timespec {
    time_t tv_sec;
    long   tv_nsec;
};

struct timeval {
    time_t tv_sec;
    long   tv_usec;
};

struct tm {
    int tm_sec;
    int tm_min;
    int tm_hour;
    int tm_mday;
    int tm_mon;
    int tm_year;
    int tm_wday;
    int tm_yday;
    int tm_isdst;
};

#define CLOCK_REALTIME           0
#define CLOCK_MONOTONIC          1
#define CLOCK_PROCESS_CPUTIME_ID 2
#define CLOCK_THREAD_CPUTIME_ID  3
#define CLOCK_MONOTONIC_RAW      4
#define CLOCK_REALTIME_COARSE    5
#define CLOCK_MONOTONIC_COARSE   6

#ifdef __cplusplus
extern "C" {
#endif

int    clock_gettime(clockid_t clockid, struct timespec *tp);
int    clock_getres(clockid_t clockid, struct timespec *res);
int    nanosleep(const struct timespec *req, struct timespec *rem);
time_t time(time_t *tloc);
int    gettimeofday(struct timeval *tv, void *tz);
struct tm *gmtime(const time_t *timep);
struct tm *localtime(const time_t *timep);
time_t mktime(struct tm *tm);
size_t strftime(char *s, size_t max, const char *format, const struct tm *tm);

#ifdef __cplusplus
}
#endif

#endif /* _TIME_H */
