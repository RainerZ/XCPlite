# multi_thread_demo — Multi-Thread Measurement

Demonstrates XCP measurement and calibration across multiple concurrent threads.
Shows thread-local event instances, shared calibration segments with safe atomic access,
and experimental context/span instrumentation for duration measurement.

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| Thread-local event instances | Each thread creates its own `DaqCreateEvent` instance |
| Thread-local measurement | Local variables registered per-thread, measured independently |
| Shared calibration segment | Single `CalSeg` accessed safely from all threads via lock/unlock |
| Consistent atomic update | Multiple parameters updated atomically across all threads |
| Context and span API | Experimental instrumentation to measure code block durations |
| Multi-thread queue contention | Performance benchmark with configurable thread count and sleep time |

### Files

| File | Purpose |
|---|---|
| `src/main.c` | Demo application — thread creation, per-thread events, shared calibration |
| `CANape/` | CANape project (A2L auto-upload, XCP UDP, port 5555) |

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

---

## CANape

Open `CANape/CANape.ini` in CANape. The project is pre-configured for XCP on UDP, port 5555,
with automatic A2L upload. If CANape cannot connect, verify the IP address in
*Device Configuration / Devices / XCP / Protocol / Transport Layer*.

---

## Performance

The lock-free queue in XCPlite is designed for minimal contention under multi-thread load.
Example statistics with 8 threads at 50 µs sleep time (MacBook Pro M3):

```
Producer acquire lock time statistics:
  count=1404480  max_spins=4  max=67583ns  avg=76ns

Lock time histogram (1404480 events):
  Range                      Count        %  Bar
  --------------------  ----------  -------  ------------------------------
  0-40ns                    120470    8.58%  #####
  40-80ns                   627770   44.70%  ##############################
  80-120ns                  528258   37.61%  #########################
  120-160ns                  64571    4.60%  ###
  ...
```

See [c_demo](../c_demo/README.md) for single-thread minimum cycle time benchmarks.
