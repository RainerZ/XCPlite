# struct_demo — Nested Struct and Array Measurement

Demonstrates how to define XCP measurement variables in nested structs,
multidimensional arrays, and arrays of structs using typedef-based type
registration. Pure measurement demo — no calibration parameters.

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| Nested struct measurement | Multi-level `typedef struct` registered as a single measurement type |
| Array of structs | Typed array where each element is a struct instance |
| Multidimensional arrays | 2D and 3D fixed-size arrays as measurement variables |
| Typedef-based type system | `A2lTypedefBegin` / `A2lTypedefEnd` for reusable measurement types |
| Pure measurement (no calibration) | No `CalSeg` — shows measurement-only instrumentation |

### Files

| File | Purpose |
|---|---|
| `src/main.c` | Demo — struct definitions, typedef registration, event loop |
| `CANape/` | CANape project (A2L auto-upload, XCP TCP, port 5555) |

---

## Building

```bash
./build.sh examples
./build/struct_demo
```

Or with CMake directly:

```bash
cmake -B build -S . -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target struct_demo
./build/struct_demo
```

---

## CANape

Open `CANape/CANape.ini` in CANape. The project is pre-configured for XCP on TCP, port 5555,
with automatic A2L upload. If CANape cannot connect, verify the IP address in
*Device Configuration / Devices / XCP / Protocol / Transport Layer*.
