/* getopt.h — GNU getopt_long extension declarations */
#ifndef _GETOPT_H
#define _GETOPT_H

#ifdef __cplusplus
extern "C" {
#endif

/* Standard getopt() is declared in <unistd.h>; this header adds the GNU
 * long-options extension used by Mesa's command-line tools. */

extern char *optarg;
extern int   optind;
extern int   opterr;
extern int   optopt;

/* no_argument / required_argument / optional_argument */
#define no_argument       0
#define required_argument 1
#define optional_argument 2

struct option {
    const char *name;
    int         has_arg;
    int        *flag;
    int         val;
};

int getopt(int argc, char *const argv[], const char *optstring);
int getopt_long(int argc, char *const argv[], const char *optstring,
                const struct option *longopts, int *longindex);
int getopt_long_only(int argc, char *const argv[], const char *optstring,
                     const struct option *longopts, int *longindex);

#ifdef __cplusplus
}
#endif

#endif /* _GETOPT_H */
