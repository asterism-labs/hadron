/* hadron_query.h — Hadron sys_query wrappers for C code (Mesa port).
 *
 * Provides typed C wrappers around the Hadron sys_query syscall so that
 * Mesa's OS abstraction layer can query CPU info and virtual memory maps
 * without using /proc/cpuinfo or /proc/self/maps.
 */
#ifndef _HADRON_QUERY_H
#define _HADRON_QUERY_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Query type codes (must match kernel/syscall/src/lib.rs) ─────────────── */

#define HADRON_QUERY_VMAPS    0x01
#define HADRON_QUERY_CPU_INFO 0x02

/* ── CpuInfo — returned by HADRON_QUERY_CPU_INFO ───────────────────────── */

/** Feature flags returned in HadronCpuInfo.features */
#define HADRON_CPU_FEAT_SSE    (1u <<  0)
#define HADRON_CPU_FEAT_SSE2   (1u <<  1)
#define HADRON_CPU_FEAT_SSE3   (1u <<  2)
#define HADRON_CPU_FEAT_SSSE3  (1u <<  3)
#define HADRON_CPU_FEAT_SSE41  (1u <<  4)
#define HADRON_CPU_FEAT_SSE42  (1u <<  5)
#define HADRON_CPU_FEAT_AVX    (1u <<  6)
#define HADRON_CPU_FEAT_AVX2   (1u <<  7)
#define HADRON_CPU_FEAT_AVX512F (1u << 8)
#define HADRON_CPU_FEAT_FMA    (1u <<  9)
#define HADRON_CPU_FEAT_AES    (1u << 10)
#define HADRON_CPU_FEAT_POPCNT (1u << 11)

/** CPU information struct filled by hadron_query_cpu_info(). */
typedef struct {
    /** Number of logical processor cores. */
    uint32_t core_count;
    /** Number of physical sockets. */
    uint32_t socket_count;
    /** CPUID feature flags (HADRON_CPU_FEAT_* bitmask). */
    uint32_t features;
    /** L1 cache size in bytes (0 = unknown). */
    uint32_t l1_cache_bytes;
    /** L2 cache size in bytes (0 = unknown). */
    uint32_t l2_cache_bytes;
    /** L3 cache size in bytes (0 = unknown). */
    uint32_t l3_cache_bytes;
    /** Cache line size in bytes. */
    uint32_t cache_line_bytes;
    uint32_t _reserved;
} HadronCpuInfo;

/* ── VmapEntry — one entry returned by HADRON_QUERY_VMAPS ─────────────── */

#define HADRON_VMAP_PROT_READ  0x1
#define HADRON_VMAP_PROT_WRITE 0x2
#define HADRON_VMAP_PROT_EXEC  0x4

/** One virtual memory mapping returned by hadron_query_vmaps(). */
typedef struct {
    /** Start address (inclusive). */
    uint64_t start;
    /** End address (exclusive). */
    uint64_t end;
    /** Protection flags (HADRON_VMAP_PROT_* bitmask). */
    uint32_t prot;
    uint32_t _pad;
    /** Human-readable name (null-terminated, may be empty). */
    char     name[32];
} HadronVmapEntry;

/* ── C wrappers ─────────────────────────────────────────────────────────── */

/**
 * Query CPU information.
 *
 * Fills `*out` on success.  Returns 0 on success, -1 on error.
 */
static inline int
hadron_query_cpu_info(HadronCpuInfo *out)
{
    /* Raw sys_query syscall: syscall number 0xF0, subid=0, buf, buflen. */
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "0"(0xF0UL),          /* syscall number */
          "D"((unsigned long)HADRON_QUERY_CPU_INFO),
          "S"(0UL),             /* sub_id */
          "d"((unsigned long)out),
          "r"(sizeof(HadronCpuInfo)) /* passed via r10 */
        : "rcx", "r11", "memory"
    );
    return (ret < 0) ? -1 : 0;
}

/**
 * Query virtual memory maps.
 *
 * Writes at most `max_entries` entries into `out`.  Returns the number of
 * entries written on success, or -1 on error.
 */
static inline int
hadron_query_vmaps(HadronVmapEntry *out, size_t max_entries)
{
    long ret;
    size_t buf_len = max_entries * sizeof(HadronVmapEntry);
    register size_t r10 __asm__("r10") = buf_len;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "0"(0xF0UL),
          "D"((unsigned long)HADRON_QUERY_VMAPS),
          "S"(0UL),
          "d"((unsigned long)out),
          "r"(r10)
        : "rcx", "r11", "memory"
    );
    if (ret < 0) return -1;
    return (int)(ret / sizeof(HadronVmapEntry));
}

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* _HADRON_QUERY_H */
