// multi_thread_demo - work implementation
//
// Mandelbrot set, rendered line by line, the line number is the work item index
// Ported from the rayon_demo of XCP-Lite Rust, inspired by the mandelbrot rayon example in the book "Programming Rust" by Jim Blandy and Jason Orendorff
//
// - The area of the complex plane to render is defined by the calibration segment 'mandelbrot' (center x, y and width)
// - A new image is rendered when the area was changed or on explicit request
// - The image is written to the file mandelbrot.png when all lines are rendered
//
// See the interface contract in work.h

#include "work.h"

#include <assert.h> // for assert
#include <math.h>   // for powf
#include <stdio.h>  // for printf
#include <stdlib.h> // for malloc, free

#include "a2l.h"    // for A2l generation
#include "xcplib.h" // for application programming interface

#include "platform.h" // for clockGetMonotonicNs

// PNG writer, single header library, public domain or MIT, see https://github.com/nothings/stb
#if defined(_MSC_VER)
#define STBI_MSC_SECURE_CRT
#endif
#define STB_IMAGE_WRITE_IMPLEMENTATION
#include "stb_image_write.h"

//-----------------------------------------------------------------------------------------------------
// Image

#define IMAGE_FILE_NAME "mandelbrot.png"
#define X_RES 2048   // Image width in pixels
#define Y_RES 1536   // Image height in pixels, number of work items in a batch
#define MAX_ITER 254 // Iteration limit, pixel value 255 marks a point in the set

// Vectorized rendering: RENDER_VECTOR_WIDTH pixels of a line are iterated together with GCC/Clang vector extensions
// The compiler maps the vector operations to the SIMD instructions of the target (e.g. 4x NEON 128 bit on ARM64 for a width of 8),
// interleaving several pixels also breaks the dependency chain of the scalar iteration, which limits the scalar loop
// Same results as the scalar variant built with the same compiler (double precision, same operations)
// Render time of the default image with 8 worker threads, -O2:
//   Apple M3:            clang scalar 242 ms, width 8 57 ms
//   Raspberry Pi 5:      clang scalar 574 ms, width 8 207 ms - gcc 12 scalar 467 ms, width 2 292 ms (gcc is slower with widths > 2 there)
// Disable by commenting out OPTION_RENDER_VECTOR, the scalar variant is used on compilers without vector extensions (e.g. MSVC)
#define OPTION_RENDER_VECTOR
#ifndef RENDER_VECTOR_WIDTH // Pixels per vector, may be overridden with -DRENDER_VECTOR_WIDTH=<n>
#ifdef __clang__
#define RENDER_VECTOR_WIDTH 8
#else
#define RENDER_VECTOR_WIDTH 2
#endif
#endif
#if defined(OPTION_RENDER_VECTOR) && (defined(__GNUC__) || defined(__clang__))
#define RENDER_VECTOR
_Static_assert(X_RES % RENDER_VECTOR_WIDTH == 0, "X_RES must be a multiple of RENDER_VECTOR_WIDTH");
#endif

//-----------------------------------------------------------------------------------------------------
// Calibration parameters

typedef struct mandelbrot {
    double x;     // Center of the area to render, real part
    double y;     // Center of the area to render, imaginary part
    double width; // Width of the area to render, the height is given by the image aspect ratio
} mandelbrot_t;

// Default parameters
// static const mandelbrot_t mandelbrot = {.x = -0.5, .y = 0.0, .width = 3.0}; // Complete set
static const mandelbrot_t mandelbrot = {.x = -0.8015, .y = 0.1561, .width = 0.0055};

// Calibration segment handle
static tXcpCalSegIndex mandelbrot_calseg = XCP_UNDEFINED_CALSEG;

//-----------------------------------------------------------------------------------------------------
// Work state

// Parameters of the current batch, a copy taken in work_begin_batch, so all lines of an image are rendered with the same parameters
static bool gValid = false; // gBatchParams contains the parameters of a rendered image
static mandelbrot_t gBatchParams;
static double gUpperLeftRe; // Upper left corner of the area to render
static double gUpperLeftIm;
static double gPixelSize; // Size of a pixel on the complex plane

// Logging of the image creation
static uint64_t gInitTime = 0;        // Time of work_init in ns, the log timestamps are relative to it
static uint64_t gRenderStartTime = 0; // Time of work_begin_batch of the current image in ns
static uint32_t gImageCount = 0;      // Number of the current image, starting with 1

// Iteration count per pixel, line by line, each line is owned by the worker which renders it
// Static (not heap), to be measurable with absolute addressing, measured as a matrix with the event batch_done
uint8_t pixels[Y_RES][X_RES];

// RGB image and color map for the PNG file
static uint8_t *gImage = NULL;
static uint8_t gColorMap[256][3];

//-----------------------------------------------------------------------------------------------------
// Coloring

// Normalize a color intensity value to the RGB range
static uint8_t normalize(float color, float factor) { return (uint8_t)(powf(color * factor, 0.8f) * 255.0f); }

// Convert a light wavelength in nm to RGB
static void wavelength_to_rgb(uint32_t i, uint8_t rgb[3]) {

    float wave = (float)i;
    float r = 0.0f, g = 0.0f, b = 0.0f;
    if (i >= 380 && i <= 439) {
        r = (440.0f - wave) / (440.0f - 380.0f);
        b = 1.0f;
    } else if (i >= 440 && i <= 489) {
        g = (wave - 440.0f) / (490.0f - 440.0f);
        b = 1.0f;
    } else if (i >= 490 && i <= 509) {
        g = 1.0f;
        b = (510.0f - wave) / (510.0f - 490.0f);
    } else if (i >= 510 && i <= 579) {
        r = (wave - 510.0f) / (580.0f - 510.0f);
        g = 1.0f;
    } else if (i >= 580 && i <= 644) {
        r = 1.0f;
        g = (645.0f - wave) / (645.0f - 580.0f);
    } else if (i >= 645 && i <= 780) {
        r = 1.0f;
    }

    float factor = 1.0f;
    if (i >= 380 && i <= 419) {
        factor = 0.3f + 0.7f * (wave - 380.0f) / (420.0f - 380.0f);
    } else if (i >= 701 && i <= 780) {
        factor = 0.3f + 0.7f * (780.0f - wave) / (780.0f - 700.0f);
    }

    rgb[0] = normalize(r, factor);
    rgb[1] = normalize(g, factor);
    rgb[2] = normalize(b, factor);
}

// Map iteration counts 0..255 to the visible spectrum, 255 (in the set) is black
static void init_color_map(void) {
    for (uint32_t i = 0; i < 256; i++) {
        wavelength_to_rgb(379 + (i * (781 - 379)) / 255, gColorMap[i]);
    }
}

//-----------------------------------------------------------------------------------------------------
// Mandelbrot set

#ifndef RENDER_VECTOR

// Number of iterations until c = (cr, ci) leaves the circle of radius 2, MAX_ITER+1 if c seems to be a member of the set
static uint8_t mandelbrot_iterations(double cr, double ci) {
    double zr = 0.0, zi = 0.0;
    for (uint32_t i = 0; i < MAX_ITER; i++) {
        double zr2 = zr * zr;
        double zi2 = zi * zi;
        if (zr2 + zi2 > 4.0) {
            return (uint8_t)i;
        }
        zi = 2.0 * zr * zi + ci;
        zr = zr2 - zi2 + cr;
    }
    return MAX_ITER + 1;
}

#else

typedef double vdouble_t __attribute__((vector_size(8 * RENDER_VECTOR_WIDTH)));
typedef int64_t vint64_t __attribute__((vector_size(8 * RENDER_VECTOR_WIDTH)));

// Iteration counts of RENDER_VECTOR_WIDTH pixels c = (cr[k], ci), same result as mandelbrot_iterations for each pixel
// A lane is active (-1) until its point left the circle of radius 2, the loop ends when no lane is active
static void mandelbrot_iterations_vector(vdouble_t cr, double ci, uint8_t *pixels) {
    vdouble_t zr = {0}, zi = {0};
    vint64_t n = {0};                    // Iteration count per lane
    vint64_t active = (vint64_t){0} - 1; // All lanes active
    for (uint32_t i = 0; i < MAX_ITER; i++) {
        vdouble_t zr2 = zr * zr;
        vdouble_t zi2 = zi * zi;
        active &= (vint64_t)(zr2 + zi2 <= 4.0); // Comparison result is -1 (true) or 0 per lane
        int64_t any = 0;
        for (int k = 0; k < RENDER_VECTOR_WIDTH; k++) {
            any |= active[k];
        }
        if (any == 0) {
            break;
        }
        n -= active; // Count the iteration for the active lanes
        zi = 2.0 * zr * zi + ci;
        zr = zr2 - zi2 + cr;
    }
    for (int k = 0; k < RENDER_VECTOR_WIDTH; k++) {
        pixels[k] = active[k] ? MAX_ITER + 1 : (uint8_t)n[k];
    }
}

#endif

//-----------------------------------------------------------------------------------------------------
// Work interface

bool work_init(void) {

    gInitTime = clockGetMonotonicNs();

    // Create a calibration segment for the area to render
    mandelbrot_calseg = XcpCreateCalSeg("mandelbrot", &mandelbrot, sizeof(mandelbrot));
    if (mandelbrot_calseg == XCP_UNDEFINED_CALSEG) {
        return false;
    }
    A2lSetSegmentAddrMode(mandelbrot_calseg, mandelbrot);
    A2lCreateParameter(mandelbrot.x, "Center of the area to render, real part", "", -2.0, 2.0);
    A2lCreateParameter(mandelbrot.y, "Center of the area to render, imaginary part", "", -2.0, 2.0);
    A2lCreateParameter(mandelbrot.width, "Width of the area to render", "", 0.0, 4.0);

    // Measure the iteration counts of the image as a matrix with the event batch_done, triggered by main.c after work_complete
    A2lSetAbsoluteAddrMode(batch_done);
    A2lCreateMeasurementMatrix(pixels, "Mandelbrot iteration count per pixel, 255 = in the set");

    // Allocate the RGB image
    gImage = (uint8_t *)malloc((size_t)X_RES * Y_RES * 3);
    if (gImage == NULL) {
        return false;
    }

    init_color_map();

#ifdef RENDER_VECTOR
    printf("Mandelbrot rendering vectorized, %d pixels per vector\n", RENDER_VECTOR_WIDTH);
#else
    printf("Mandelbrot rendering scalar\n");
#endif
    return true;
}

uint32_t work_begin_batch(bool requested) {

    // Take a copy of the parameters for this batch
    const mandelbrot_t *params = (const mandelbrot_t *)XcpLockCalSeg(mandelbrot_calseg);
    mandelbrot_t p = *params;
    XcpUnlockCalSeg(mandelbrot_calseg);

    // Render a new image only if requested, or for the first image, or if the area was changed
    if (!requested && gValid && p.x == gBatchParams.x && p.y == gBatchParams.y && p.width == gBatchParams.width) {
        return 0;
    }
    const char *reason = requested ? "requested" : gValid ? "parameters changed" : "first image";
    gBatchParams = p;
    gValid = true;

    // Pixel size and upper left corner of the area on the complex plane
    gPixelSize = p.width / X_RES;
    gUpperLeftRe = p.x - p.width / 2.0;
    gUpperLeftIm = p.y + gPixelSize * Y_RES / 2.0;

    gImageCount++;
    gRenderStartTime = clockGetMonotonicNs();
    printf("[%9.3f s] Image %u: rendering started (%s), x=%g, y=%g, width=%g, %dx%d pixels\n", (double)(gRenderStartTime - gInitTime) / 1e9, gImageCount, reason, p.x, p.y, p.width,
           X_RES, Y_RES);
    return Y_RES;
}

void work_execute(uint32_t line) {

    assert(line < Y_RES);

    // Render one line of the image
    uint8_t *row = pixels[line];
    double ci = gUpperLeftIm - (double)line * gPixelSize;
#ifdef RENDER_VECTOR
    for (uint32_t column = 0; column < X_RES; column += RENDER_VECTOR_WIDTH) {
        vdouble_t cr;
        for (int k = 0; k < RENDER_VECTOR_WIDTH; k++) {
            cr[k] = gUpperLeftRe + (double)(column + (uint32_t)k) * gPixelSize;
        }
        mandelbrot_iterations_vector(cr, ci, row + column);
    }
#else
    for (uint32_t column = 0; column < X_RES; column++) {
        double cr = gUpperLeftRe + (double)column * gPixelSize;
        row[column] = mandelbrot_iterations(cr, ci);
    }
#endif
}

bool work_complete(uint32_t lines) {

    assert(lines == Y_RES);
    (void)lines;

    // Convert iteration counts to RGB and write the PNG file
    uint64_t t1 = clockGetMonotonicNs();
    const uint8_t *p = &pixels[0][0];
    for (size_t i = 0; i < (size_t)X_RES * Y_RES; i++) {
        const uint8_t *rgb = gColorMap[p[i]];
        gImage[i * 3 + 0] = rgb[0];
        gImage[i * 3 + 1] = rgb[1];
        gImage[i * 3 + 2] = rgb[2];
    }
    bool ok = stbi_write_png(IMAGE_FILE_NAME, X_RES, Y_RES, 3, gImage, X_RES * 3) != 0;
    uint64_t t2 = clockGetMonotonicNs();

    // Render time is from work_begin_batch until all lines were rendered and the main loop detected completion (includes up to one main loop cycle)
    double render_ms = (double)(t1 - gRenderStartTime) / 1e6;
    double write_ms = (double)(t2 - t1) / 1e6;
    double total_ms = (double)(t2 - gRenderStartTime) / 1e6;
    if (!ok) {
        printf("[%9.3f s] Image %u: failed to write %s\n", (double)(t2 - gInitTime) / 1e9, gImageCount, IMAGE_FILE_NAME);
        return false;
    }
    printf("[%9.3f s] Image %u: created %s in %.1f ms (render %.1f ms, color+PNG write %.1f ms)\n", (double)(t2 - gInitTime) / 1e9, gImageCount, IMAGE_FILE_NAME, total_ms,
           render_ms, write_ms);
    return true;
}

void work_shutdown(void) {
    free(gImage);
    gImage = NULL;
}
