# hello_xcp_cpp — C++ API Example

XCPlite example using the C++ API. Demonstrates the same features as
[hello_xcp](../hello_xcp/README.md) in idiomatic C++ with the RAII calibration wrapper
and the variadic macro/template API.

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| XCP server startup | `XcpInit` + `XcpEthServerInit` with runtime A2L generation |
| C++ RAII calibration wrapper | `CalSeg<ParametersT>` — locked/unlocked via RAII on scope exit |
| Global variable measurement | `DaqRegisterVar` on global variables |
| Stack and heap variable measurement | Local variables and heap-allocated instances |
| Class instance member measurement | `DaqRegisterMember` on member variables |
| Variadic macro/template API | `XCP_VAR(...)`, `XCP_CAL(...)` — compact all-in-one registration |
| Function instrumentation | Measure local variables and parameters inside a member function |

### Files

| File | Purpose |
|---|---|
| `src/main.cpp` | Demo application — server setup, calibration segment, events, measurement |
| `CANape/` | CANape project (A2L auto-upload, XCP UDP, port 5555) |

---

## Building

```bash
./build.sh examples
./build/hello_xcp_cpp
```

Or with CMake directly:

```bash
cmake -B build -S . -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target hello_xcp_cpp
./build/hello_xcp_cpp
```

---

## CANape

Open `CANape/CANape.ini` in CANape. The project is pre-configured for XCP on UDP, port 5555,
with automatic A2L upload. If CANape cannot connect, verify the IP address in
*Device Configuration / Devices / XCP / Protocol / Transport Layer*.

![CANape Screenshot](CANape.png)

---

## Next steps

More advanced topics are covered by the other examples:

- **c_demo** — complex calibration objects (maps, curves), consistent atomic updates, polling
- **multi_thread_demo** — thread-local events, shared calibration, context/span instrumentation
- **cpp_demo** — signal generators, lookup tables with shared axis, two class instances
- **struct_demo** — nested structs, arrays of structs, typedef-based measurement types

