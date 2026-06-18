# hello_xcp — Basic C API Example

The simplest XCPlite example in pure C. The recommended starting point.
For C++ usage see [hello_xcp_cpp](../hello_xcp_cpp/README.md).

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| XCP server startup | `XcpInit` + `XcpEthServerInit` with runtime A2L generation |
| Calibration segment | `CalSegDecl` / `CalSegCreate`, safe read via `CalSegLock` / `CalSegUnlock` |
| Global variable measurement | `DaqRegisterVar` on global variables |
| Stack variable measurement | `DaqRegisterVar` on local variables inside a function |
| Addressing modes | Direct address, relative-to-base, and pointer-based registration |
| Function instrumentation | Register function parameters and local variables; trigger event inside function |

### Files

| File | Purpose |
|---|---|
| `src/main.c` | Demo application — server setup, calibration segment, events, measurement |
| `CANape/` | CANape project (A2L auto-upload, XCP UDP, port 5555) |

---

## Building

```bash
./build.sh examples
./build/hello_xcp
```

Or with CMake directly:

```bash
cmake -B build -S . -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target hello_xcp
./build/hello_xcp
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

- **hello_xcp_cpp** — C++ RAII calibration segment wrapper, variadic macro API
- **c_demo** — complex calibration objects (maps, curves), consistent atomic updates, polling
- **multi_thread_demo** — thread-local events, shared calibration, context/span instrumentation
- **cpp_demo** — signal generators, lookup tables, class instance measurement
- **struct_demo** — nested structs, arrays of structs, typedef-based measurement types

