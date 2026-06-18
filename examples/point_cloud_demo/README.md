# point_cloud_demo — 3D Point Cloud Visualization

Demonstrates how to visualize dynamic data structures in the CANape 3D scene window.
Simulates a 3D point cloud with simple physics — bouncing points inside a boundary box —
and measures the entire array of point structs as an XCP measurement variable.

---

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| Array-of-struct measurement | `std::array<PointT, N>` measured as a typed array |
| Calibration parameters | `ParametersT` — max points, boundary size, velocity, radius tunable at runtime |
| 3D scene visualization | CANape 3D scene window displays the live point cloud |
| C++ API | `CalSeg<ParametersT>`, `DaqCreateEvent`, `DaqRegisterMember` |

### Files

| File | Purpose |
|---|---|
| `src/main.cpp` | Point cloud simulation — physics, XCP server setup, event loop |
| `CANape/` | CANape project with 3D scene window pre-configured |

---

## Building

```bash
./build.sh examples
./build/point_cloud_demo
```

Or with CMake directly:

```bash
cmake -B build -S . -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target point_cloud_demo
./build/point_cloud_demo
```

---

## CANape

Open `CANape/CANape.ini` in CANape. The project is pre-configured for XCP on TCP, port 5555,
with automatic A2L upload and a 3D scene window for the point cloud visualization.

If CANape cannot connect, verify the IP address in
*Device Configuration / Devices / XCP / Protocol / Transport Layer*.
