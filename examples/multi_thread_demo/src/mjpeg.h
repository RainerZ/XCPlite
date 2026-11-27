#pragma once

// multi_thread_demo - MJPEG HTTP server
//
// Streams RGB frames as MJPEG (multipart/x-mixed-replace with one JPEG image per frame) over HTTP, any browser shows it as video:
//   http://<host>:<port>/        HTML page with the stream
//   http://<host>:<port>/stream  MJPEG stream
//
// One server thread, one client at a time, a new connection is served when the previous one is closed.
// The application publishes frames from any thread, mjpeg_publish copies the frame. The server thread encodes and sends the newest
// frame, frames are dropped when encoding or the client is slower than the application. A new client gets the newest frame at once.
// Uses the socket and thread abstraction of xcplib, the sockets must be initialized (XcpEthServerInit).

#include <stdbool.h> // for bool
#include <stdint.h>  // for uintxx_t

#ifdef __cplusplus
extern "C" {
#endif

/// Start the MJPEG HTTP server
/// @param port TCP port of the HTTP server
/// @param width Width of the frames in pixels
/// @param height Height of the frames in pixels
/// @param quality JPEG quality 1..100
/// @return false if the server could not be started (e.g. the port is in use)
bool mjpeg_start(uint16_t port, uint32_t width, uint32_t height, int quality);

/// Publish a new frame, RGB with 3 bytes per pixel, width * height pixels line by line, the frame is copied
void mjpeg_publish(const uint8_t *rgb);

/// Stop the server and release its resources
void mjpeg_shutdown(void);

#ifdef __cplusplus
}
#endif
