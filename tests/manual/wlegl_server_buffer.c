// Stage 1 smoke test for server-side android_wlegl buffer allocation.
//
// Build and run inside the tawc session:
//
//   wayland-scanner client-header compositor/protocols/android_wlegl.xml \
//       /tmp/android_wlegl-client.h
//   wayland-scanner private-code compositor/protocols/android_wlegl.xml \
//       /tmp/android_wlegl-protocol.c
//   cc tests/manual/wlegl_server_buffer.c /tmp/android_wlegl-protocol.c \
//       -I/tmp -lwayland-client -o /tmp/wlegl-server-buffer
//   /tmp/wlegl-server-buffer
//
// Expected output: one or more buffer_fd events, an ints array, a wl_buffer
// with a sane format/stride, and (if the compositor sends it) a
// buffer_linear_layout. A compositor that did not implement
// get_server_buffer_handle would instead kill this client with a protocol
// error ("server-side buffer allocation is not supported").

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <wayland-client.h>

#include "android_wlegl-client.h"

static struct android_wlegl *wlegl;
static int fds;
static int ints;
static int buffers;
static int layouts;
static int32_t buffer_format = -1;
static int32_t buffer_stride = -1;

static void registry_global(void *data, struct wl_registry *registry,
                            uint32_t name, const char *interface,
                            uint32_t version) {
    (void)data;
    if (strcmp(interface, "android_wlegl") == 0) {
        uint32_t bind_version = version < 2 ? version : 2;
        wlegl = wl_registry_bind(registry, name, &android_wlegl_interface,
                                 bind_version);
        fprintf(stderr, "bound android_wlegl v%u\n", bind_version);
    }
}

static void registry_remove(void *data, struct wl_registry *registry,
                            uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {
    registry_global,
    registry_remove,
};

static void handle_fd(void *data, struct android_wlegl_server_buffer_handle *h,
                      int32_t fd) {
    (void)data;
    (void)h;
    fds++;
    fprintf(stderr, "buffer_fd: %d\n", fd);
}

static void handle_ints(void *data,
                        struct android_wlegl_server_buffer_handle *h,
                        struct wl_array *array) {
    (void)data;
    (void)h;
    ints++;
    fprintf(stderr, "buffer_ints: %zu bytes (%zu ints)\n", array->size,
            array->size / sizeof(int32_t));
}

static void handle_buffer(void *data,
                          struct android_wlegl_server_buffer_handle *h,
                          struct wl_buffer *buffer, int32_t format,
                          int32_t stride) {
    (void)data;
    (void)h;
    buffers++;
    buffer_format = format;
    buffer_stride = stride;
    fprintf(stderr, "buffer: %p format=%d stride=%d\n", (void *)buffer, format,
            stride);
    wl_buffer_destroy(buffer);
}

static void handle_layout(void *data,
                          struct android_wlegl_server_buffer_handle *h,
                          uint32_t format, uint32_t stride) {
    (void)data;
    (void)h;
    layouts++;
    fprintf(stderr, "buffer_linear_layout: format=%u stride=%u\n", format,
            stride);
}

static const struct android_wlegl_server_buffer_handle_listener handle_listener =
    {
        handle_fd,
        handle_ints,
        handle_buffer,
        handle_layout,
};

int main(void) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) {
        fprintf(stderr, "wl_display_connect failed\n");
        return 1;
    }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip failed\n");
        return 1;
    }
    if (!wlegl) {
        fprintf(stderr, "compositor does not advertise android_wlegl\n");
        return 1;
    }

    struct android_wlegl_server_buffer_handle *handle =
        android_wlegl_get_server_buffer_handle(wlegl, 1205, 602, 1, 0x333);
    if (!handle) {
        fprintf(stderr, "get_server_buffer_handle returned NULL\n");
        return 1;
    }
    android_wlegl_server_buffer_handle_add_listener(handle, &handle_listener,
                                                    NULL);
    if (wl_display_roundtrip(display) < 0) {
        fprintf(stderr, "roundtrip failed (protocol error?)\n");
        return 1;
    }
    android_wlegl_server_buffer_handle_destroy(handle);

    fprintf(stderr,
            "summary: fds=%d ints=%d buffers=%d layouts=%d format=%d "
            "stride=%d\n",
            fds, ints, buffers, layouts, buffer_format, buffer_stride);

    int ok = fds > 0 && ints > 0 && buffers == 1 && buffer_stride >= 1205;
    fprintf(stderr, "%s\n", ok ? "PASS" : "FAIL");
    wl_display_disconnect(display);
    return ok ? 0 : 1;
}
