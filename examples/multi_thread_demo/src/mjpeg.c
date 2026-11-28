// multi_thread_demo - MJPEG HTTP server
//
// See the interface in mjpeg.h
//
// Two threads:
//   - Encoder thread: encodes each published frame into the static buffer jpeg (JPEG_MAX_SIZE bytes) and triggers the XCP event
//     'jpeg', which measures the JPEG image (jpeg, jpeg_size, jpeg_image), also without a browser connected
//   - HTTP thread: accepts one client at a time and sends the newest JPEG image as MJPEG stream
// The buffer jpeg is protected by jpeg_mutex: written by the encoder thread while encoding and triggering the event, copied by
// the HTTP thread for sending.

#include "mjpeg.h"

#include <stdio.h>  // for printf, snprintf
#include <stdlib.h> // for malloc, free
#include <string.h> // for memcpy, strncmp

#include "a2l.h"      // for A2L generation
#include "platform.h" // for THREAD_HANDLE, create_thread, join_thread, MUTEX, mutexInit, sleepUs
#include "sockets.h"  // for socketOpen, socketBind, socketListen, socketAccept, socketRecv, socketSend, socketClose
#include "xcplib.h"   // for DaqCreateEvent, DaqTriggerEvent

// JPEG encoder, the implementation is in work.c
#include "stb_image_write.h"

#define JPEG_MAX_SIZE (256 * 1024)    // Maximum size of a JPEG image, a larger image is dropped (800x600, quality 75: 10..148 KB)
_Static_assert(JPEG_MAX_SIZE % 1024 == 0, "JPEG_MAX_SIZE must be a multiple of the row size 1024");
#define MJPEG_REQUEST_TIMEOUT_MS 2000 // Timeout for receiving the HTTP request
#define MJPEG_POLL_US 2000            // Poll interval for a new frame or a new JPEG image
#define MJPEG_SEND_CHUNK 16384        // socketSend sends at most 32767 bytes

static const char kPage[] = "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
                            "<!DOCTYPE html><html><head><title>multi_thread_demo</title></head>"
                            "<body style=\"margin:0;background:#000\"><img src=\"/stream\" style=\"display:block;max-width:100%;margin:auto\"></body></html>";
static const char kStreamHeader[] = "HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
static const char kNotFound[] = "HTTP/1.0 404 Not Found\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nNot found\r\n";

// The JPEG image, measured with the event 'jpeg' (static, absolute addressing), protected by gMjpeg.jpeg_mutex
// The buffer is a matrix of JPEG_ROW_SIZE byte rows: the dimensions of an array in A2L (MATRIX_DIM) are 16 bit values
#define JPEG_ROW_SIZE 1024
static uint8_t jpeg[JPEG_MAX_SIZE / JPEG_ROW_SIZE][JPEG_ROW_SIZE]; // JPEG image, the first jpeg_size bytes are valid
static uint8_t *const jpeg_bytes = &jpeg[0][0];                    // The same buffer as bytes
static uint32_t jpeg_size = 0;      // Size of the JPEG image in bytes
static uint32_t jpeg_image = 0;     // Number of the image of the application (mjpeg_publish)

static struct {
    volatile bool run;
    SOCKET_HANDLE listen_socket;
    volatile SOCKET_HANDLE client_socket; // Connected client, shut down by mjpeg_shutdown to wake a blocked send
    THREAD_HANDLE encoder_thread;
    THREAD_HANDLE http_thread;
    uint32_t width, height;
    int quality;

    // Newest frame, published by the application, protected by frame_mutex
    MUTEX frame_mutex;
    uint8_t *frame;
    uint32_t frame_count; // Number of frames published, 0 = no frame yet
    uint32_t frame_image; // Image number of the newest frame

    // JPEG image, protected by jpeg_mutex
    MUTEX jpeg_mutex;
    uint32_t jpeg_count; // Number of JPEG images encoded, 0 = no image yet

    // Encoder thread only
    uint8_t *encode_frame; // Copy of the newest frame for encoding
    bool jpeg_overflow;    // The current JPEG image is larger than JPEG_MAX_SIZE
    uint32_t dropped;      // Number of JPEG images dropped, larger than JPEG_MAX_SIZE

    // HTTP thread only
    uint8_t *send_jpeg; // Copy of the newest JPEG image for sending
} gMjpeg = {.listen_socket = INVALID_SOCKET_HANDLE, .client_socket = INVALID_SOCKET_HANDLE};

//-----------------------------------------------------------------------------------------------------
// Encoder thread

// stb_image_write output callback, appends to the JPEG buffer, an image larger than JPEG_MAX_SIZE is marked as overflow
static void jpeg_write(void *context, void *data, int size) {
    (void)context;
    if (gMjpeg.jpeg_overflow || jpeg_size + (uint32_t)size > JPEG_MAX_SIZE) {
        gMjpeg.jpeg_overflow = true;
        return;
    }
    memcpy(jpeg_bytes + jpeg_size, data, (size_t)size);
    jpeg_size += (uint32_t)size;
}

// Encode each new frame and trigger the event jpeg, frames published while encoding are dropped
static THREAD_FUNC_RETURN encoder_thread(void *p) {
    (void)p;
    uint32_t encoded_count = 0;
    while (gMjpeg.run) {

        // Wait for a new frame
        mutexLock(&gMjpeg.frame_mutex);
        uint32_t count = gMjpeg.frame_count;
        uint32_t image = gMjpeg.frame_image;
        if (count != 0 && count != encoded_count) {
            memcpy(gMjpeg.encode_frame, gMjpeg.frame, (size_t)gMjpeg.width * gMjpeg.height * 3);
        }
        mutexUnlock(&gMjpeg.frame_mutex);
        if (count == 0 || count == encoded_count) {
            sleepUs(MJPEG_POLL_US);
            continue;
        }
        encoded_count = count;

        // Encode into the buffer jpeg and trigger the event jpeg, the DAQ copy is done in the trigger, the buffer is consistent
        mutexLock(&gMjpeg.jpeg_mutex);
        jpeg_size = 0;
        gMjpeg.jpeg_overflow = false;
        bool ok = stbi_write_jpg_to_func(jpeg_write, NULL, (int)gMjpeg.width, (int)gMjpeg.height, 3, gMjpeg.encode_frame, gMjpeg.quality) != 0;
        if (ok && !gMjpeg.jpeg_overflow) {
            jpeg_image = image;
            gMjpeg.jpeg_count++;
            if (gMjpeg.jpeg_count == 0) {
                gMjpeg.jpeg_count = 1; // 0 means no image
            }
            DaqTriggerEvent(jpeg);
        } else {
            jpeg_size = 0;
            if (gMjpeg.dropped++ == 0) {
                if (ok) {
                    printf("MJPEG: image %u dropped, the JPEG image is larger than %u bytes, further drops are not reported\n", image, (unsigned)JPEG_MAX_SIZE);
                } else {
                    printf("MJPEG: image %u dropped, JPEG encoding failed, further drops are not reported\n", image);
                }
            }
        }
        mutexUnlock(&gMjpeg.jpeg_mutex);
    }
    THREAD_FUNC_END;
}

//-----------------------------------------------------------------------------------------------------
// HTTP thread

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

// Send the MJPEG stream until the client closes the connection or the server is stopped
static void send_stream(SOCKET_HANDLE client) {
    if (!send_all(client, (const uint8_t *)kStreamHeader, sizeof(kStreamHeader) - 1)) {
        return;
    }
    uint32_t sent_count = 0;
    while (gMjpeg.run) {

        // Wait for a JPEG image newer than the last one sent, a new client gets the newest image at once
        mutexLock(&gMjpeg.jpeg_mutex);
        uint32_t count = gMjpeg.jpeg_count;
        uint32_t size = jpeg_size;
        if (count != 0 && count != sent_count) {
            memcpy(gMjpeg.send_jpeg, jpeg_bytes, size);
        }
        mutexUnlock(&gMjpeg.jpeg_mutex);
        if (count == 0 || count == sent_count) {
            sleepUs(MJPEG_POLL_US);
            continue;
        }
        sent_count = count;

        // Send, images encoded in the meantime are dropped
        char header[128];
        int header_size = snprintf(header, sizeof(header), "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: %u\r\n\r\n", (unsigned)size);
        if (!send_all(client, (const uint8_t *)header, (size_t)header_size) || !send_all(client, gMjpeg.send_jpeg, size) || !send_all(client, (const uint8_t *)"\r\n", 2)) {
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

// HTTP thread: accept and handle one client at a time
// mjpeg_shutdown wakes a blocked accept by shutting down (Linux) and closing (macOS, Windows) the listening socket,
// the thread uses its own copy of the handle
static THREAD_FUNC_RETURN http_thread(void *p) {
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

//-----------------------------------------------------------------------------------------------------

bool mjpeg_start(uint16_t port, uint32_t width, uint32_t height, int quality) {
    gMjpeg.width = width;
    gMjpeg.height = height;
    gMjpeg.quality = quality;
    gMjpeg.frame = (uint8_t *)malloc((size_t)width * height * 3);
    gMjpeg.encode_frame = (uint8_t *)malloc((size_t)width * height * 3);
    gMjpeg.send_jpeg = (uint8_t *)malloc(JPEG_MAX_SIZE);
    if (gMjpeg.frame == NULL || gMjpeg.encode_frame == NULL || gMjpeg.send_jpeg == NULL) {
        return false;
    }
    mutexInit(&gMjpeg.frame_mutex, false, 0);
    mutexInit(&gMjpeg.jpeg_mutex, false, 0);

    // Instrumentation: event jpeg and the JPEG image with absolute addressing
    DaqCreateEvent(jpeg);
    A2lSetAbsoluteAddrMode(jpeg);
    A2lCreateMeasurement(jpeg_size, "Size of the JPEG image in bytes");
    A2lCreateMeasurement(jpeg_image, "Number of the image");
    A2lCreateMeasurementMatrix(jpeg, "JPEG image, the first jpeg_size bytes are valid");

    static const uint8_t any_addr[4] = {0, 0, 0, 0}; // All interfaces
    if (!socketOpen(&gMjpeg.listen_socket, SOCKET_MODE_TCP | SOCKET_MODE_REUSEADDR) || !socketBind(gMjpeg.listen_socket, any_addr, port) ||
        !socketListen(gMjpeg.listen_socket)) {
        printf("MJPEG: HTTP server on port %u could not be started\n", port);
        socketClose(&gMjpeg.listen_socket);
        return false;
    }

    static SOCKET_HANDLE thread_listen_socket; // The copy of the handle for the HTTP thread
    thread_listen_socket = gMjpeg.listen_socket;
    gMjpeg.run = true;
    if (create_thread(&gMjpeg.encoder_thread, NULL, encoder_thread, NULL) != 0) {
        gMjpeg.run = false;
        socketClose(&gMjpeg.listen_socket);
        return false;
    }
    if (create_thread(&gMjpeg.http_thread, NULL, http_thread, &thread_listen_socket) != 0) {
        gMjpeg.run = false;
        join_thread(gMjpeg.encoder_thread);
        socketClose(&gMjpeg.listen_socket);
        return false;
    }
    printf("MJPEG stream %ux%u on http://localhost:%u (or the IP address of this host)\n", width, height, port);
    return true;
}

void mjpeg_publish(const uint8_t *rgb, uint32_t image) {
    if (!gMjpeg.run) {
        return;
    }
    mutexLock(&gMjpeg.frame_mutex);
    memcpy(gMjpeg.frame, rgb, (size_t)gMjpeg.width * gMjpeg.height * 3);
    gMjpeg.frame_image = image;
    gMjpeg.frame_count++;
    if (gMjpeg.frame_count == 0) {
        gMjpeg.frame_count = 1; // 0 means no frame
    }
    mutexUnlock(&gMjpeg.frame_mutex);
}

void mjpeg_shutdown(void) {
    if (gMjpeg.run) {
        // Stop the threads: the encoder and stream loops check run, a blocked accept or send is woken by shutting down the sockets
        // A blocked accept returns on shutdown on Linux and on close on macOS and Windows
        gMjpeg.run = false;
        socketShutdown(gMjpeg.client_socket);
        socketShutdown(gMjpeg.listen_socket);
        socketClose(&gMjpeg.listen_socket);
        join_thread(gMjpeg.http_thread);
        join_thread(gMjpeg.encoder_thread);
        mutexDestroy(&gMjpeg.frame_mutex);
        mutexDestroy(&gMjpeg.jpeg_mutex);
        if (gMjpeg.dropped > 0) {
            printf("MJPEG: %u images dropped (larger than %u bytes or encoding failed)\n", gMjpeg.dropped, (unsigned)JPEG_MAX_SIZE);
        }
    }
    free(gMjpeg.frame);
    free(gMjpeg.encode_frame);
    free(gMjpeg.send_jpeg);
    gMjpeg.frame = gMjpeg.encode_frame = gMjpeg.send_jpeg = NULL;
}
