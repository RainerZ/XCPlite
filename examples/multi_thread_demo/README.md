# multi_thread_demo — Worker Threads

Demonstrates triggering the same event from multiple threads.  

The example use case shows measurement of work distributed over a fixed set of worker threads.
The main loop starts batches of work items; worker threads fetch work items from a shared
lock-free counter, execute them and report completion. A single global event, triggered by
any worker, measures each completed work item.

The example work renders the Mandelbrot set line by line, the line number is the work item index. It is ported from the `rayon_demo` of [XCP-Lite Rust](https://github.com/vectorgrp/xcp-lite),
inspired by the Mandelbrot example in the book "Programming Rust" by Jim Blandy and Jason Orendorff.

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| Event triggered from multiple threads | One global event `work_done`, triggered by whichever worker completed a work item |
| Stack addressing across threads | Worker local variables registered once, captured from the stack frame of the triggering thread |
| Lock-free work distribution | Shared atomic counter, fetched with a compare-and-swap loop, no thread pool library |
| Batch completion detection | Separate atomic completion counter with release/acquire ordering |
| Calibration as external request | Main loop asks the work for a batch when `params.start_request` changes, or cyclically (`params.auto_restart`), the work decides if there is something to do |
| Parallel speedup | Sum of work item durations / batch runtime |
| Uneven work item cost | Lines crossing the Mandelbrot set take more iterations, balanced by fetching one line at a time |
| Generic work interface | The work is defined in `work.c` behind the interface `work.h`, `main.c` only knows work item indices |


## Observe the Mandelbrot image generation, measure large arrays

The example work renders images of the Mandelbrot set. The area is defined by in calibration segment `mandelbrot`: the demo starts
with the area of `mandelbrot.x`, `mandelbrot.y` and `mandelbrot.width` and continuously zooms into the center. The file
`mandelbrot.png` is written for the first image. Each image is compressed to JPEG (800 x 600) and streamed as Motion JPEG video by
an HTTP server on port 8080.

The uncompressed image (`pixels`, 1.92 MB) and the compressed image (`jpeg`, a buffer of 256 KB) can be measured with XCP. The compressed image can be measured with the OPTION_MTU 8000 only. The default settings in `src/xcplib_cfg.h` (OPTION_MTU 1500 and about 7 KB DAQ memory) are not sufficient. The uncompressed image needs about 50 KB DAQ memory. Both need the maximum DTO size set in the current default configuration (`XCPTL_MAX_DTO_SIZE = (XCPTL_MAX_SEGMENT_SIZE - 8)` ). An array has to fit into one DAQ list of at most 251 ODTs, with `XCPTL_MAX_DTO_SIZE` 1024 this is 256016 bytes.

See the full list of events, measurements and calibration parameters below.


---

## Building

```bash
./build.sh examples
./build/multi_thread_demo
```

Or with CMake directly:

```bash
cmake -B build -S . -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target multi_thread_demo
./build/multi_thread_demo
```

Measure the rendering of the images:

```bash
xcpclient --udp --upload-a2l --mea worker_id work_index work_time work_latency batch_speedup --time 5
```


Measure the JPEG image of the video (event `jpeg`) to an MDF file:

```bash
xcpclient --udp --upload-a2l --mea jpeg jpeg_size jpeg_image --time 5 --mdf jpeg.mf4
```

`jpeg` is a buffer of 256 KB (a matrix of 256 rows of 1024 bytes, array dimensions in A2L are 16 bit values), the first
`jpeg_size` bytes are the JPEG image, `jpeg_image` is the number of the image. The complete buffer is measured, a JPEG image of
800 x 600 pixels has 10 to 150 KB. A JPEG image larger than 256 KB is dropped (not sent, not measured), the console reports it.


Measure the uncompressed image (`pixels`, event `batch_done`) to an MDF file. This is not possible with the default settings of
the library, increase `OPTION_DAQ_MEM_SIZE` in `src/xcplib_cfg.h` to 64 KB (`(1024 * 64)`) first:

```bash
xcpclient --udp --upload-a2l --mea pixels --time 5 --mdf pixels.mf4
```


Render single images and write `mandelbrot.png`: switch continuous mode off, then change the area:

```bash
xcpclient --udp --upload-a2l --cal mandelbrot.continuous 0
xcpclient --udp --upload-a2l --cal mandelbrot.width 0.02
```



### Watch the Mandelbrot video in the browser

The demo streams each image as MJPEG video over HTTP, without XCP: open `http://localhost:8080` (or the IP address of the host) in a
browser to watch the zoom animation. The video has half the resolution (800 x 600, the mean color of each 2 x 2 pixel block, JPEG
quality 75). An encoder thread encodes each image, also without a browser connected, and triggers the XCP event `jpeg`, which
measures the JPEG image. The HTTP server sends the newest JPEG image, images are dropped when encoding or the browser is slower than
rendering. One browser at a time, a new connection is served when the previous one is closed.
`http://localhost:8080/stream` is the MJPEG stream itself. Configured with `OPTION_HTTP_PORT` in `work.c` (0 disables the server).

Start the demo and watch the video:

1. Build and start the demo (a release build renders faster):

   ```bash
   ./build.sh release examples
   ./build/multi_thread_demo
   ```

   The console shows `MJPEG stream 800x600 on http://localhost:8080` and one line for each image.

2. Open `http://localhost:8080` in a browser. It shows the zoom animation: the zoom into the center (`mandelbrot.x`,
   `mandelbrot.y`) by the factor `mandelbrot.zoom` per image, about 10 % per second at 18 images per second. After
   `mandelbrot.images` images (1730) it starts again at `mandelbrot.width`. The console line of each image shows its number in
   the zoom sequence, `Image 1731 (zoom 1): ...`.
   On another computer use the IP address of the host running the demo, for example `http://192.168.0.206:8080`.

3. Change the animation while it runs, with xcpclient or CANape (calibration window), for example zoom faster, zoom out, a
   shorter sequence, or another area. The parameters stay set in the demo, xcpclient may be closed:

   ```bash
   xcpclient --udp --upload-a2l --cal mandelbrot.zoom 0.98
   xcpclient --udp --upload-a2l --cal mandelbrot.zoom 1.006
   xcpclient --udp --upload-a2l --cal mandelbrot.images 500
   xcpclient --udp --upload-a2l --cal mandelbrot.width 3.0
   ```

   Stop the animation with `--cal mandelbrot.continuous 0`, the browser shows the image of the calibrated area again,
   `--cal mandelbrot.continuous 1` starts it again.

4. Stop the demo with Ctrl+C.

---

## CANape

Open `CANape/CANape.ini` in CANape. The project is pre-configured for XCP on UDP, port 5555,
with automatic A2L upload. If CANape cannot connect, verify the IP address in
*Device Configuration / Devices / XCP / Protocol / Transport Layer*.

The browser windows show the MJPEG video and the PNG file of the first image.

CANape Screenshot:




---

## Files

| File | Purpose |
|---|---|
| `src/main.c` | Generic part — worker threads, work distribution, batch control, instrumentation |
| `src/work.h` | Work interface and threading contract |
| `src/work.c` | Work implementation — Mandelbrot set, replace with the real work |
| `src/mjpeg.h`, `src/mjpeg.c` | MJPEG HTTP server: encodes the images to JPEG (event `jpeg`) and streams them as video to a browser |
| `src/stb_image_write.h` | PNG and JPEG writer, single header library from [stb](https://github.com/nothings/stb), public domain or MIT |
| `CANape/` | CANape project (A2L auto-upload, XCP UDP, port 5555) |

---

## How it works

```
main loop                                   worker threads (THREAD_COUNT)
---------                                   -----------------------------
work_init()
start_request changed or auto_restart:      loop:
  n = work_begin_batch()                      fetch_work(&index):  CAS gWorkRemaining n -> n-1 if n > 0
  gWorkDone = 0                               if fetched:
  gWorkRemaining = n  (release)                 work_execute(index)
                                                trigger event work_done
cyclic:                                         gWorkDone++  (release)
  gWorkDone == n (acquire)?                   else:
    -> work_complete(n)                         sleep poll_us
    -> trigger event batch_done
  trigger event mainloop
work_shutdown()
```

- Work item indices are handed out in ascending order `0..n-1`, `n` is the number of work items returned by `work_begin_batch`.
- A plain `atomic_fetch_sub` is not used, because idle workers polling an empty counter would underflow it.
- `gWorkRemaining == 0` means all work items are fetched, not completed. Completion is tracked by `gWorkDone`.
- A new batch is only started when the previous batch is completed.
- Idle workers poll with `params.poll_us`, so `work_latency` includes up to one poll interval.

### Work interface (`work.h`)

| Function | Called by | Purpose |
|---|---|---|
| `bool work_init(void)` | main thread, once, before the worker threads are started | Create calibration segments, register parameters and measurements, allocate resources |
| `uint32_t work_begin_batch(void)` | main thread, no batch running, on request (`start_request`) or cyclic with `auto_restart` | Define the work items of a new batch and prepare their input data, return the number of work items (0 = no batch) |
| `void work_execute(uint32_t index)` | worker threads, concurrently, each index exactly once | Execute one work item |
| `bool work_complete(uint32_t items)` | main thread, after all work items are executed | Evaluate the batch result, false counts as batch error |
| `void work_shutdown(void)` | main thread, after the worker threads are joined | Release resources |

Data written in `work_begin_batch` is visible in `work_execute`, data written in `work_execute` is visible in `work_complete`.
Concurrent work items may write to data owned by their index (e.g. `result[index]`) without synchronization.

### Mandelbrot work (`work.c`)

| Function | Mandelbrot |
|---|---|
| `work_init` | Calibration segment `mandelbrot`, measurement of the image `pixels`, RGB image and color map, start of the MJPEG HTTP server |
| `work_begin_batch` | Copy the calibration parameters for the whole batch, compute the area on the complex plane, return the number of lines. A new image is only rendered for the first image, when the parameters changed or in continuous mode |
| `work_execute(line)` | Render one line: iteration count per pixel, at most 254 iterations, 255 marks a point in the set. Vectorized with GCC/Clang vector extensions (`OPTION_RENDER_VECTOR`, `RENDER_VECTOR_WIDTH` pixels at once), scalar on other compilers |
| `work_complete` | Map the iteration counts to colors of the visible spectrum and write `mandelbrot.png` to the current directory (the first image, or each image when continuous mode is off), publish the 2 x 2 downscaled image to the MJPEG server |
| `work_shutdown` | Stop the MJPEG HTTP server, free the buffers |

The resolution is `X_RES` x `Y_RES` = 1600 x 1200 (`#define` in `work.c`). Each line is written only by the worker which renders it, no synchronization needed.
The image is measured as one array with the event `batch_done`: 1920000 bytes, 242 ODTs of one DAQ list with `XCPTL_MAX_DTO_SIZE` 7960 (`OPTION_MTU` 8000, at most 251 ODTs), about 50 KB DAQ memory (`OPTION_DAQ_MEM_SIZE`, the default of 16 KB is not sufficient).
At 2048 x 1536, rendering takes about 57 ms with 8 worker threads (242 ms scalar), coloring and writing the PNG file about 125 ms on the main thread (MacBook Pro M3, clang, release build).
On a Raspberry Pi 5 rendering takes 207 ms with clang (vector width 8) and 292 ms with gcc 12 (vector width 2, gcc is slower with wider vectors there).
The console shows one line for each image: in continuous mode the number of the image in the zoom sequence, the area, the render time and, if the PNG file is written, the time for coloring and writing it.

### Calibration parameters (`params`, `mandelbrot`)

| Parameter | Default | Description |
|---|---|---|
| `start_request` | 0 | Increment to ask the work for a new batch (useful without `auto_restart`) |
| `auto_restart` | true | Ask the work for a new batch cyclically while no batch is running, the work decides if there is something to do |
| `poll_us` | 1000 | Worker poll interval when there is nothing to do |
| `cycle_us` | 10000 | Main loop cycle time |
| `mandelbrot.x` | -0.8015 | Center of the area to render, real part |
| `mandelbrot.y` | 0.1561 | Center of the area to render, imaginary part |
| `mandelbrot.width` | 0.0055 | Width of the area to render, the height follows from the aspect ratio (-0.5, 0.0, 3.0 shows the complete set) |
| `mandelbrot.zoom` | 0.99416 | Zoom factor per image in continuous mode, the width is multiplied by it for each image (0.99416 = 0.9^(1/18), about 10 % per second at 18 images per second, 1 = no zoom, > 1 zooms out). The sequence of images does not depend on the frame rate |
| `mandelbrot.images` | 1730 | Images of a zoom sequence in continuous mode, then the zoom restarts at `mandelbrot.width` (1730 images zoom to a width of about 2e-7). The zoom also restarts when the area is changed or the width gets smaller than 1e-10 (double precision), 0 = restart at this limit only |
| `mandelbrot.continuous` | true | Render a new image in every batch (with `params.auto_restart`), also when the area was not changed, zooming with `mandelbrot.zoom`. The PNG file is written for the first image only |

### Measurements

| Event | Variable | Description |
|---|---|---|
| `work_done` | `worker_id` | Index of the worker thread which completed the work item |
| `work_done` | `work_index` | Index of the completed work item |
| `work_done` | `work_time` | Duration of the work item (us) |
| `work_done` | `work_latency` | Time from batch start to start of the work item (us) |
| `mainloop` | `batch_running` | Batch is running |
| `mainloop` | `work_remaining`, `work_done` | Progress of the current batch |
| `batch_done` | `batch_count` | Number of completed batches |
| `batch_done` | `batch_errors` | Number of batches for which `work_complete` returned false |
| `batch_done` | `batch_size` | Number of work items of the batch |
| `batch_done` | `batch_time` | Runtime of the batch until all work items were completed (us) |
| `batch_done` | `batch_speedup` | Sum of work item durations / `batch_time` |
| `batch_done` | `batch_duration` | Overall duration of the batch, from batch start until `work_complete` returned (us) |
| `batch_done` | `pixels` | The image: iteration count per pixel, 255 = in the set, 1600 x 1200 bytes (needs `OPTION_DAQ_MEM_SIZE` 64 KB) |
| `jpeg` | `jpeg` | The JPEG image of the video, a buffer of 256 KB (256 rows of 1024 bytes), the first `jpeg_size` bytes are valid |
| `jpeg` | `jpeg_size` | Size of the JPEG image in bytes |
| `jpeg` | `jpeg_image` | Number of the image, to correlate with `batch_done` |

