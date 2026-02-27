#!/usr/bin/env bash
# sysroot-setup.sh — Assemble the Hadron cross-compilation sysroot for Mesa.
#
# Usage:
#   ./ports/mesa/sysroot-setup.sh [REPO_ROOT] [SYSROOT_DIR]
#
# Defaults:
#   REPO_ROOT   = directory two levels above this script (project root)
#   SYSROOT_DIR = $REPO_ROOT/build/mesa-sysroot
#
# After this script completes, the sysroot will contain:
#   <sysroot>/usr/include/   — hadron-libc public headers
#   <sysroot>/usr/lib/       — libhadron_libc.a, libcompiler_rt.a
#   <sysroot>/usr/lib/pkgconfig/wayland-client.pc  — stub pkg-config
#
# The Meson cross-file (ports/mesa/hadron.cross) is updated in-place with
# the resolved absolute sysroot path so Mesa's build directory can be placed
# anywhere.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${1:-"$(cd "$SCRIPT_DIR/../.." && pwd)"}"
SYSROOT_DIR="${2:-"$REPO_ROOT/build/mesa-sysroot"}"

echo "[sysroot-setup] REPO_ROOT   = $REPO_ROOT"
echo "[sysroot-setup] SYSROOT_DIR = $SYSROOT_DIR"

# ── Locate libhadron_libc.a ──────────────────────────────────────────────────
#
# gluon builds userspace crates for x86_64-unknown-hadron-user; the output
# archive lives under target/<target>/release/ or target/<target>/debug/.

LIBC_A=""
for profile in release debug; do
    candidate="$REPO_ROOT/target/x86_64-unknown-hadron-user/$profile/libhadron_libc.a"
    if [[ -f "$candidate" ]]; then
        LIBC_A="$candidate"
        echo "[sysroot-setup] Found libhadron_libc.a at $LIBC_A"
        break
    fi
done

if [[ -z "$LIBC_A" ]]; then
    echo "[sysroot-setup] libhadron_libc.a not found — building hadron-libc now..."
    (cd "$REPO_ROOT" && just build 2>&1 | tail -5)
    for profile in release debug; do
        candidate="$REPO_ROOT/target/x86_64-unknown-hadron-user/$profile/libhadron_libc.a"
        if [[ -f "$candidate" ]]; then
            LIBC_A="$candidate"
            break
        fi
    done
    if [[ -z "$LIBC_A" ]]; then
        echo "[sysroot-setup] ERROR: libhadron_libc.a still not found after build."
        exit 1
    fi
fi

# ── Locate compiler-rt builtins ──────────────────────────────────────────────
#
# We ship compiler-rt as a vendored Rust-compiled rlib or as a pre-built
# archive.  For Mesa's C compilation the linker needs a compiler_rt that
# provides __divti3, __udivti3, etc.  If not present we create a thin stub.

COMPILER_RT_A=""
for candidate in \
    "$REPO_ROOT/target/x86_64-unknown-hadron-user/release/libcompiler_rt.a" \
    "$REPO_ROOT/vendor/compiler-rt/libclang_rt.builtins-x86_64.a"; do
    if [[ -f "$candidate" ]]; then
        COMPILER_RT_A="$candidate"
        break
    fi
done

# ── Create sysroot directories ───────────────────────────────────────────────

mkdir -p \
    "$SYSROOT_DIR/usr/include/sys" \
    "$SYSROOT_DIR/usr/lib" \
    "$SYSROOT_DIR/usr/lib/pkgconfig"

# ── Copy headers ─────────────────────────────────────────────────────────────

INCLUDE_SRC="$REPO_ROOT/userspace/hadron-libc/include"
cp -r "$INCLUDE_SRC"/. "$SYSROOT_DIR/usr/include/"
echo "[sysroot-setup] Copied headers from $INCLUDE_SRC"

# ── Copy libraries ────────────────────────────────────────────────────────────

cp "$LIBC_A" "$SYSROOT_DIR/usr/lib/libhadron_libc.a"
echo "[sysroot-setup] Installed libhadron_libc.a"

if [[ -n "$COMPILER_RT_A" ]]; then
    cp "$COMPILER_RT_A" "$SYSROOT_DIR/usr/lib/libcompiler_rt.a"
    echo "[sysroot-setup] Installed libcompiler_rt.a"
else
    echo "[sysroot-setup] WARNING: compiler-rt not found; creating empty stub."
    ar rcs "$SYSROOT_DIR/usr/lib/libcompiler_rt.a"
fi

# ── Wayland pkg-config stub ───────────────────────────────────────────────────
#
# Mesa's Meson build checks for wayland-client via pkg-config.  We provide a
# minimal stub that points at our sysroot headers and avoids the system
# Wayland installation being picked up by the host pkg-config.

cat > "$SYSROOT_DIR/usr/lib/pkgconfig/wayland-client.pc" << EOF
prefix=$SYSROOT_DIR/usr
exec_prefix=\${prefix}
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: wayland-client
Description: Hadron stub — wayland-client header for Mesa WSI
Version: 1.22.0
Cflags: -I\${includedir}
Libs: -L\${libdir}
EOF

cat > "$SYSROOT_DIR/usr/lib/pkgconfig/wayland-server.pc" << EOF
prefix=$SYSROOT_DIR/usr
exec_prefix=\${prefix}
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: wayland-server
Description: Hadron stub — wayland-server header for Mesa WSI
Version: 1.22.0
Cflags: -I\${includedir}
Libs: -L\${libdir}
EOF

cat > "$SYSROOT_DIR/usr/lib/pkgconfig/wayland-egl.pc" << EOF
prefix=$SYSROOT_DIR/usr
exec_prefix=\${prefix}
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: wayland-egl
Description: Hadron stub — wayland-egl header for Mesa WSI
Version: 18.1.0
Cflags: -I\${includedir}
Libs: -L\${libdir}
EOF

echo "[sysroot-setup] Generated Wayland pkg-config stubs"

# ── Wayland protocol headers stub ─────────────────────────────────────────────
#
# Mesa's WSI includes <wayland-client-protocol.h> which is normally generated
# by wayland-scanner.  Provide a minimal stub with the types Mesa expects.

WAYLAND_H="$SYSROOT_DIR/usr/include/wayland-client.h"
WAYLAND_PROTO_H="$SYSROOT_DIR/usr/include/wayland-client-protocol.h"

if [[ ! -f "$WAYLAND_H" ]]; then
cat > "$WAYLAND_H" << 'EOF'
/* wayland-client.h — Hadron stub for Mesa Wayland WSI */
#ifndef WAYLAND_CLIENT_H
#define WAYLAND_CLIENT_H
#include <wayland-client-protocol.h>

struct wl_display;
struct wl_event_queue;

struct wl_display *wl_display_connect(const char *name);
struct wl_display *wl_display_connect_to_fd(int fd);
void wl_display_disconnect(struct wl_display *display);
int  wl_display_get_fd(struct wl_display *display);
int  wl_display_dispatch(struct wl_display *display);
int  wl_display_dispatch_queue(struct wl_display *display, struct wl_event_queue *queue);
int  wl_display_dispatch_pending(struct wl_display *display);
int  wl_display_flush(struct wl_display *display);
int  wl_display_roundtrip(struct wl_display *display);

#endif /* WAYLAND_CLIENT_H */
EOF
fi

if [[ ! -f "$WAYLAND_PROTO_H" ]]; then
cat > "$WAYLAND_PROTO_H" << 'EOF'
/* wayland-client-protocol.h — Hadron minimal stub for Mesa Wayland WSI */
#ifndef WAYLAND_CLIENT_PROTOCOL_H
#define WAYLAND_CLIENT_PROTOCOL_H

#include <stdint.h>
#include <stddef.h>

struct wl_interface {
    const char *name;
    int         version;
    int         method_count;
    const void *methods;
    int         event_count;
    const void *events;
};

struct wl_proxy;
struct wl_object { const struct wl_interface *interface; const void *implementation; uint32_t id; };
struct wl_array { size_t size; size_t alloc; void *data; };

typedef void (*wl_dispatcher_func_t)(const void *data, void *target, uint32_t opcode,
                                     const struct wl_message *message, union wl_argument *args);
typedef void (*wl_log_func_t)(const char *fmt, ...);

struct wl_message { const char *name; const char *signature; const struct wl_interface **types; };

struct wl_proxy   *wl_proxy_create(struct wl_proxy *factory, const struct wl_interface *interface);
void               wl_proxy_destroy(struct wl_proxy *proxy);
int                wl_proxy_add_listener(struct wl_proxy *proxy, void (**implementation)(void), void *data);
void               wl_proxy_set_user_data(struct wl_proxy *proxy, void *user_data);
void              *wl_proxy_get_user_data(struct wl_proxy *proxy);
uint32_t           wl_proxy_get_version(struct wl_proxy *proxy);
uint32_t           wl_proxy_get_id(struct wl_proxy *proxy);
const char        *wl_proxy_get_class(struct wl_proxy *proxy);
void               wl_proxy_marshal(struct wl_proxy *p, uint32_t opcode, ...);
struct wl_proxy   *wl_proxy_marshal_constructor(struct wl_proxy *proxy, uint32_t opcode,
                                                 const struct wl_interface *interface, ...);
struct wl_proxy   *wl_proxy_marshal_constructor_versioned(struct wl_proxy *proxy, uint32_t opcode,
                                                           const struct wl_interface *interface,
                                                           uint32_t version, ...);

/* Core globals */
extern const struct wl_interface wl_display_interface;
extern const struct wl_interface wl_registry_interface;
extern const struct wl_interface wl_compositor_interface;
extern const struct wl_interface wl_surface_interface;
extern const struct wl_interface wl_buffer_interface;
extern const struct wl_interface wl_shm_interface;
extern const struct wl_interface wl_shm_pool_interface;
extern const struct wl_interface wl_callback_interface;
extern const struct wl_interface wl_seat_interface;
extern const struct wl_interface wl_output_interface;
extern const struct wl_interface wl_region_interface;
extern const struct wl_interface wl_subcompositor_interface;
extern const struct wl_interface wl_subsurface_interface;

#endif /* WAYLAND_CLIENT_PROTOCOL_H */
EOF
fi

echo "[sysroot-setup] Generated Wayland stub headers"

# ── xdg-shell protocol header stub ───────────────────────────────────────────

XDG_SHELL_H="$SYSROOT_DIR/usr/include/xdg-shell-client-protocol.h"
if [[ ! -f "$XDG_SHELL_H" ]]; then
cat > "$XDG_SHELL_H" << 'EOF'
/* xdg-shell-client-protocol.h — Hadron stub for Mesa XDG Shell WSI */
#ifndef XDG_SHELL_CLIENT_PROTOCOL_H
#define XDG_SHELL_CLIENT_PROTOCOL_H

#include <wayland-client-protocol.h>

extern const struct wl_interface xdg_wm_base_interface;
extern const struct wl_interface xdg_surface_interface;
extern const struct wl_interface xdg_toplevel_interface;
extern const struct wl_interface xdg_popup_interface;
extern const struct wl_interface xdg_positioner_interface;

#endif /* XDG_SHELL_CLIENT_PROTOCOL_H */
EOF
fi

echo "[sysroot-setup] Generated xdg-shell stub header"

# ── Patch cross-file with resolved sysroot path ──────────────────────────────

CROSS_FILE="$SCRIPT_DIR/hadron.cross"
if [[ -f "$CROSS_FILE" ]]; then
    # Replace all SYSROOT_PATH placeholder occurrences (c_args, link_args, sys_root)
    sed -i.bak "s|SYSROOT_PATH|$SYSROOT_DIR|g" "$CROSS_FILE"
    rm -f "$CROSS_FILE.bak"
    echo "[sysroot-setup] Updated all SYSROOT_PATH entries in $CROSS_FILE"
fi

echo ""
echo "[sysroot-setup] Done.  Sysroot assembled at: $SYSROOT_DIR"
echo "[sysroot-setup] Next: run ports/mesa/build-mesa.sh"
