/* termios.h — Terminal I/O for Hadron libc (stubs) */
#ifndef _TERMIOS_H
#define _TERMIOS_H

typedef unsigned int tcflag_t;
typedef unsigned char cc_t;
typedef unsigned int speed_t;

#define NCCS 32

struct termios {
    tcflag_t c_iflag;
    tcflag_t c_oflag;
    tcflag_t c_cflag;
    tcflag_t c_lflag;
    cc_t     c_cc[NCCS];
    speed_t  c_ispeed;
    speed_t  c_ospeed;
};

/* tcsetattr actions */
#define TCSANOW   0
#define TCSADRAIN 1
#define TCSAFLUSH 2

/* c_lflag bits */
#define ECHO   0x0008
#define ICANON 0x0002
#define ISIG   0x0001

int    tcgetattr(int fd, struct termios *termios_p);
int    tcsetattr(int fd, int action, const struct termios *termios_p);
speed_t cfgetispeed(const struct termios *termios_p);
speed_t cfgetospeed(const struct termios *termios_p);
int    cfsetispeed(struct termios *termios_p, speed_t speed);
int    cfsetospeed(struct termios *termios_p, speed_t speed);

#endif /* _TERMIOS_H */
