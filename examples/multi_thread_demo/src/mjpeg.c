// multi_thread_demo - MJPEG HTTP server
//
// See the interface in mjpeg.h

#include "mjpeg.h"

#include <stdio.h>  // for printf, snprintf
#include <stdlib.h> // for malloc, realloc, free
#include <string.h> // for memcpy, strncmp

#include "platform.h" // for THREAD_HANDLE, create_thread, join_thread, MUTEX, mutexInit, sleepUs
#include "sockets.h"  // for socketOpen, socketBind, socketListen, socketAccept, socketRecv, socketSend, socketClose

// JPEG encoder, the implementation is in work.c
#include "stb_image_write.h"

#define MJPEG_REQUEST_TIMEOUT_MS 2000 // Timeout for receiving the HTTP request
#define MJPEG_POLL_US 2000            // Poll interval for a new frame
#define MJPEG_SEND_CHUNK 16384        // socketSend sends at most 32767 bytes

static const char kPage[] = "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
                            "<!DOCTYPE html><html><head><title>multi_thread_demo</title></head>"
                            "<body style=\"margin:0;background:#000\"><img src=\"/stream\" style=\"display:block;max-width:100%;margin:auto\"></body></html>";
static const char kStreamHeader[] = "HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
static const char kNotFound[] = "HTTP/1.0 404 Not Found\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nNot found\r\n";

static struct {
    volatile bool run;
    SOCKET_HANDLE listen_socket;
    volatile SOCKET_HANDLE client_socket; // Connected client, shut down by mjpeg_shutdown to wake a blocked send
    THREAD_HANDLE thread;
    uint32_t width, height;
    int quality;

    // Newest frame, published by the application, protected by mutex
    MUTEX mutex;
    uint8_t *frame;
    uint32_t frame_count; // Number of frames published, 0 = no frame yet

    // Server thread only
    uint8_t *encode_frame; // Copy of the newest frame for encoding
    uint8_t *jpeg;         // Encoded JPEG
    size_t jpeg_size;
    size_t jpeg_capacity;
} gMjpeg = {.listen_socket = INVALID_SOCKET_HANDLE, .client_socket = INVALID_SOCKET_HANDLE};

// Send all bytes, socketSend may send less and returns at most 32767
static bool send_all(SOCKET_HANDLE socket, const uint8_t *data, size_t size) {
    while (size > 0) {
        uint16_t n = (uint16_t)(size < MJPEG_SEND_CHUNK ? size : MJPEG_SEND_CHUNK);
        int16_t sent = socketSend(socket, data, n);
        if (sent <= 0) {
            return false; // Closed by the client or error
        }
        data += sent;
        size -= (size_t)sent;
    }
    return true;
}

// stb_image_write output callback, appends to the JPEG buffer
static void jpeg_write(void *context, void *data, int size) {
    (void)context;
    if (gMjpeg.jpeg_size + (size_t)size > gMjpeg.jpeg_capacity) {
        size_t capacity = (gMjpeg.jpeg_size + (size_t)size) * 2;
        uint8_t *jpeg = (uint8_t *)realloc(gMjpeg.jpeg, capacity);
        if (jpeg == NULL) {
            return; // Out of memory, the frame is truncated
        }
        gMjpeg.jpeg = jpeg;
        gMjpeg.jpeg_capacity = capacity;
    }
    memcpy(gMjpeg.jpeg + gMjpeg.jpeg_size, data, (size_t)size);
    gMjpeg.jpeg_size += (size_t)size;
}

// Send the MJPEG stream until the client closes the connection or the server is stopped
static void send_stream(SOCKET_HANDLE client) {
    if (!send_all(client, (const uint8_t *)kStreamHeader, sizeof(kStreamHeader) - 1)) {
        return;
    }
    uint32_t sent_count = 0;
    while (gMjpeg.run) {

        // Wait for a frame newer than the last one sent, a new client gets the newest frame at once
        mutexLock(&gMjpeg.mutex);
        uint32_t count = gMjpeg.frame_count;
        if (count != 0 && count != sent_count) {
            memcpy(gMjpeg.encode_frame, gMjpeg.frame, (size_t)gMjpeg.width * gMjpeg.height * 3);
        }
        mutexUnlock(&gMjpeg.mutex);
        if (count == 0 || count == sent_count) {
            sleepUs(MJPEG_POLL_US);
            continue;
        }
        sent_count = count;

        // Encode and send, frames published in the meantime are dropped
        gMjpeg.jpeg_size = 0;
        if (!stbi_write_jpg_to_func(jpeg_write, NULL, (int)gMjpeg.width, (int)gMjpeg.height, 3, gMjpeg.encode_frame, gMjpeg.quality)) {
            printf("MJPEG: JPEG encoding failed\n");
            return;
        }
        char header[128];
        int header_size = snprintf(header, sizeof(header), "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: %u\r\n\r\n", (unsigned)gMjpeg.jpeg_size);
        if (!send_all(client, (const uint8_t *)header, (size_t)header_size) || !send_all(client, gMjpeg.jpeg, gMjpeg.jpeg_size) ||
            !send_all(client, (const uint8_t *)"\r\n", 2)) {
            return; // Closed by the client
        }
    }
}

// Handle one HTTP connection: GET / (page), GET /stream (MJPEG stream), anything else 404
static void handle_client(SOCKET_HANDLE client) {
    char request[1024];
    socketSetTimeout(client, MJPEG_REQUEST_TIMEOUT_MS);
    int16_t n = socketRecv(client, (uint8_t *)request, sizeof(request) - 1, false);
    if (n <= 0) {
        return;
    }
    request[n] = 0;
    if (strncmp(request, "GET /stream ", 12) == 0) {
        send_stream(client);
    } else if (strncmp(request, "GET / ", 6) == 0) {
        send_all(client, (const uint8_t *)kPage, sizeof(kPage) - 1);
    } else {
        send_all(client, (const uint8_t *)kNotFound, sizeof(kNotFound) - 1);
    }
}

// Server thread: accept and handle one client at a time
// mjpeg_shutdown wakes a blocked accept by shutting down (Linux) and closing (macOS, Windows) the listening socket,
// the thread uses its own copy of the handle
static THREAD_FUNC_RETURN mjpeg_thread(void *p) {
    SOCKET_HANDLE listen_socket = *(SOCKET_HANDLE *)p;
    while (gMjpeg.run) {
        SOCKET_HANDLE client = socketAccept(listen_socket, NULL);
        if (client == INVALID_SOCKET_HANDLE) {
            continue; // Error or shutdown
        }
        gMjpeg.client_socket = client;
        handle_client(client);
        gMjpeg.client_socket = INVALID_SOCKET_HANDLE;
        socketShutdown(client);
        socketClose(&client);
    }
    THREAD_FUNC_END;
}

bool mjpeg_start(uint16_t port, uint32_t width, uint32_t height, int quality) {
    gMjpeg.width = width;
    gMjpeg.height = height;
    gMjpeg.quality = quality;
    gMjpeg.frame = (uint8_t *)malloc((size_t)width * height * 3);
    gMjpeg.encode_frame = (uint8_t *)malloc((size_t)width * height * 3);
    if (gMjpeg.frame == NULL || gMjpeg.encode_frame == NULL) {
        return false;
    }
    mutexInit(&gMjpeg.mutex, false, 0);

    static const uint8_t any_addr[4] = {0, 0, 0, 0}; // All interfaces
    if (!socketOpen(&gMjpeg.listen_socket, SOCKET_MODE_TCP | SOCKET_MODE_REUSEADDR) || !socketBind(gMjpeg.listen_socket, any_addr, port) ||
        !socketListen(gMjpeg.listen_socket)) {
        printf("MJPEG: HTTP server on port %u could not be started\n", port);
        socketClose(&gMjpeg.listen_socket);
        return false;
    }

    static SOCKET_HANDLE thread_listen_socket; // The copy of the handle for the server thread
    thread_listen_socket = gMjpeg.listen_socket;
    gMjpeg.run = true;
    if (create_thread(&gMjpeg.thread, NULL, mjpeg_thread, &thread_listen_socket) != 0) {
        gMjpeg.run = false;
        socketClose(&gMjpeg.listen_socket);
        return false;
    }
    printf("MJPEG stream %ux%u on http://localhost:%u (or the IP address of this host)\n", width, height, port);
    return true;
}

void mjpeg_publish(const uint8_t *rgb) {
    if (!gMjpeg.run) {
        return;
    }
    mutexLock(&gMjpeg.mutex);
    memcpy(gMjpeg.frame, rgb, (size_t)gMjpeg.width * gMjpeg.height * 3);
    gMjpeg.frame_count++;
    if (gMjpeg.frame_count == 0) {
        gMjpeg.frame_count = 1; // 0 means no frame
    }
    mutexUnlock(&gMjpeg.mutex);
}

void mjpeg_shutdown(void) {
    if (gMjpeg.run) {
        // Stop the server thread: the stream loop checks run, a blocked accept or send is woken by shutting down the sockets
        // A blocked accept returns on shutdown on Linux and on close on macOS and Windows
        gMjpeg.run = false;
        socketShutdown(gMjpeg.client_socket);
        socketShutdown(gMjpeg.listen_socket);
        socketClose(&gMjpeg.listen_socket);
        join_thread(gMjpeg.thread);
        mutexDestroy(&gMjpeg.mutex);
    }
    free(gMjpeg.frame);
    free(gMjpeg.encode_frame);
    free(gMjpeg.jpeg);
    gMjpeg.frame = gMjpeg.encode_frame = gMjpeg.jpeg = NULL;
}
