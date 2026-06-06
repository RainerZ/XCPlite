# c_demo — Complex Calibration Objects and Async Access

Shows more complex data objects (structs, arrays) and calibration objects (axes, maps and curves).
Demonstrates asynchronous polling access to stack variables, consistent atomic parameter updates,
and calibration page switching.

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| Maps, curves, lookup tables | Calibration parameters with fixed and shared axes |
| Struct and array measurement | Global and stack variables of structured types |
| Asynchronous stack polling | CANape reads `counter` on the stack via polling (no DAQ event) |
| Consistent atomic update | `test_byte1` and `test_byte2` updated atomically via indirect calibration mode |
| Calibration page switching | Switch between default and RAM calibration page at runtime |
| EPK version check | Version string verified by CANape on connect |
| Minimum cycle time benchmark | `delay_us` calibration parameter controls main loop sleep |

### Files

| File | Purpose |
|---|---|
| `src/main.c` | Demo application — calibration objects, async polling, event loop |
| `CANape/` | CANape project (A2L auto-upload, XCP UDP, port 5555, indirect cal mode) |

---

## Building

```bash
./build.sh examples
./build/c_demo
```

Or with CMake directly:

```bash
cmake -B build -S . -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target c_demo
./build/c_demo
```

> **Note:** This example enables log level 4 (`OPTION_LOG_LEVEL 4`) so XCP commands are
> visible on the console — useful for understanding how async access and consistent updates work.

---

## CANape

Open `CANape/CANape.ini` in CANape. The project is pre-configured for XCP on UDP, port 5555,
with automatic A2L upload and indirect calibration mode enabled.

If CANape cannot connect, verify the IP address in
*Device Configuration / Devices / XCP / Protocol / Transport Layer*.

---

## Key concepts

### Asynchronous access to stack variable

`counter` lives on the stack. CANape is configured to poll it once per second
(`Polling every second` measurement mode) — CANape actively reads the value during its lifetime
rather than waiting for a DAQ trigger event. The variable also has write access and can be
modified from the calibration window.

### Consistent parameter changes

`test_byte1` and `test_byte2` are in the same calibration segment. The application asserts
`test_byte1 == -test_byte2` and prints a warning if they are ever inconsistent.
The *indirect calibration mode* (enabled in the CANape toolbar) ensures both values are applied
atomically — the application never sees a partial update.

### Minimum cycle time

The `delay_us` calibration parameter controls the main loop sleep time. Reducing it probes the
minimum measurement cycle time the system can sustain without queue overruns. XCPlite's
lock-free queue allows sub-10 µs cycles on typical Linux/macOS hardware.

Example lock-time statistics at 2 µs sleep time (MacBook Pro M3):

```
Producer acquire lock time statistics:
  count=3032642  max_spins=0  max=29250ns  avg=27ns

Lock time histogram (3032642 events):
  Range                      Count        %  Bar
  --------------------  ----------  -------  ------------------------------
  0-40ns                   1233151   40.66%  #####################
  40-80ns                  1682263   55.47%  ##############################
  80-120ns                   89802    2.96%  #
  ...
```



