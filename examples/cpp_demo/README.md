# cpp_demo — XCPlite C++ API Demo

Demonstrates the C++ API of XCPlite including the calibration parameter segment RAII wrapper,
measurement of member variables in class instances, signal generators with lookup tables, and
the variadic all-in-one macro API.

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| C++ calibration RAII wrapper | `CalSeg<ParametersT>` — segment locked/unlocked via RAII on scope exit |
| Measurement in class instances | `SignalGenerator` member variables measured via `DaqRegisterMember` |
| Stack variable measurement | Local variables registered and measured within member functions |
| Signal generator with lookup table | `LookupTableT` — 2D calibration map with shared axis |
| Variadic macro API | `XCP_VAR(...)`, `XCP_CAL(...)` — compact registration in one call |
| Two independent instances | Two `SignalGenerator` instances with separate events and parameter sets |

### Files

| File | Purpose |
|---|---|
| `src/main.cpp` | Demo application — server setup, event loop, two signal generator instances |
| `src/sig_gen.hpp/cpp` | `SignalGenerator` class — measures sine/square/saw waves |
| `src/lookup.hpp/cpp` | `LookupTableT` — 2D lookup table with shared axis as calibration parameter |
| `CANape/` | CANape project (A2L auto-upload, XCP UDP, port 5555) |
| `CANape_25/` | CANape 25 project variant |

---

## Building

```bash
./build.sh examples
./build/cpp_demo
```

Or with CMake directly:

```bash
cmake -B build -S . -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target cpp_demo
./build/cpp_demo
```

---

## CANape

Open `CANape/CANape.ini` in CANape. The project is pre-configured for XCP on UDP, port 5555,
with automatic A2L upload. If CANape cannot connect, verify the IP address in
*Device Configuration / Devices / XCP / Protocol / Transport Layer*.

> **Note:** If `CANAPE_24` is defined in `sig_gen.hpp`, the lookup table uses a nested typedef
> with `THIS.` references to its shared axis.
