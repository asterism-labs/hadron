/* sys/statvfs.h — Filesystem statistics for Hadron libc (POSIX.1-2001) */
#ifndef _SYS_STATVFS_H
#define _SYS_STATVFS_H

#include <bits/features.h>
#include <sys/types.h>

#define ST_RDONLY 0x0001
#define ST_NOSUID 0x0002

struct statvfs {
    unsigned long f_bsize;
    unsigned long f_frsize;
    unsigned long f_blocks;
    unsigned long f_bfree;
    unsigned long f_bavail;
    unsigned long f_files;
    unsigned long f_ffree;
    unsigned long f_favail;
    unsigned long f_fsid;
    unsigned long f_flag;
    unsigned long f_namemax;
    unsigned int  _padding[6];
};

#ifdef __cplusplus
extern "C" {
#endif

int statvfs(const char *path, struct statvfs *buf);
int fstatvfs(int fd, struct statvfs *buf);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_STATVFS_H */
