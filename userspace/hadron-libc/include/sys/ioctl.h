/* sys/ioctl.h — Device control for Hadron libc */
#ifndef _SYS_IOCTL_H
#define _SYS_IOCTL_H

#include <bits/features.h>

int ioctl(int fd, unsigned long cmd, ...);

/* Terminal ioctl requests */
#define TCGETS    0x5401
#define TCSETS    0x5402
#define TIOCGWINSZ 0x5413

struct winsize {
    unsigned short ws_row;
    unsigned short ws_col;
    unsigned short ws_xpixel;
    unsigned short ws_ypixel;
};

#endif /* _SYS_IOCTL_H */
