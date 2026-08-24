# OPTION_ENABLE_UDP_RAW — Raw-Ethernet XCP/UDP Transport

## Motivation

XCPlite currently supports XCP-on-Ethernet via the OS socket API (`OPTION_ENABLE_UDP` /
`OPTION_ENABLE_TCP`).  Some embedded targets have no TCP/IP stack at all and
expose only a raw Ethernet send/receive interface (direct EMAC driver, or an
RTOS Ethernet abstraction without lwIP).  This feature adds a build variant
that implements the XCP UDP/IP transport entirely within XCPlite, sitting
directly on top of raw Ethernet frames.

Future backends that motivate the abstraction design:
- **Vector XLAPI** (Windows) — raw frame access on Vector VN-interface hardware
- **ASAM CMP** (Capture Module Protocol) — XCP encapsulated in CMP Ethernet frames

---

## Compilation Guard

`OPTION_ENABLE_UDP_RAW` is **mutually exclusive** with `OPTION_ENABLE_UDP` and
`OPTION_ENABLE_TCP`.  Define exactly one transport option per build.

**Intended build configurations:**
- `rtos` configuration (FreeRTOS bare-metal targets, primary use case)
- `default` configuration on **Windows** (Vector XLAPI backend, future)

**Not intended** for the `default` Linux/macOS configuration: those rely on
`socketSendToV` / `socketSendV` (scatter-gather via `sendmsg`) for efficient
DAQ transmission, which the raw variant does not and will not implement.

Activate by setting in an `xcplib_*_cfg.h` override file:

```c
#undef  OPTION_ENABLE_UDP
#undef  OPTION_ENABLE_TCP
#define OPTION_ENABLE_UDP_RAW
```

---

## API Subset

Only the following functions from `sockets.h` are implemented; all others are
absent from the build:

| Function | Notes |
|---|---|
| `socketStartup` | initialize raw Ethernet HAL |
| `socketCleanup` | release HAL resources |
| `socketGetErrorString` | human-readable error string |
| `socketOpen` | allocate a raw socket context (UDP only) |
| `socketBind` | set local IP address and UDP port; triggers ARP |
| `socketShutdown` | unblock a blocked receive |
| `socketClose` | release the socket context |
| `socketRecvFrom` | receive one UDP datagram, strip Ethernet/IP/UDP headers |
| `socketSendTo` | prepend Ethernet/IP/UDP headers and transmit |
| `socketSetTimeout` | configure receive timeout |

TCP, multicast (`socketJoin`), scatter-gather (`socketSendToV`, `socketSendV`),
hardware timestamps, and `socketGetLocalAddr` are **not** part of this variant.

---

## Architecture

```
xcpethtl.c / xcpethserver.c
        │  sockets.h API
        ▼
  socket_raw.c
    ├── UDP/IP layer   (hand-crafted IP + UDP header construction/parsing)
    ├── ARP            (minimal: send gratuitous ARP, parse first reply for peer MAC)
    └── Raw Ethernet HAL
              │
    ┌─────────┴──────────────────────────┐
    │ Linux (testing)   Embedded target  │
    │ AF_PACKET socket  EMAC / ETH HAL   │
    └────────────────────────────────────┘
```

### SOCKET_HANDLE type

**Step 1–3 (Linux / int fd):**  
`SOCKET_HANDLE` is the same `int` raw socket fd as in the normal POSIX variant.

**Step 2+ (embedded):**  
`SOCKET_HANDLE` becomes a pointer to `struct socket_raw` which carries all
state needed without an OS socket API:

```c
struct socket_raw {
    int       fd;           // raw fd (Linux) or HAL handle
    uint8_t   src_mac[6];   // local MAC (read from HAL or configured)
    uint8_t   dst_mac[6];   // peer MAC (filled by ARP)
    uint8_t   src_ip[4];    // local IP
    uint8_t   dst_ip[4];    // peer IP (XCP master)
    uint16_t  src_port;     // local UDP port
    uint16_t  dst_port;     // peer UDP port
    bool      peer_known;   // ARP completed
};
```

This change is introduced in Step 2 and does not require any changes to
callers in `xcpethtl.c` because `SOCKET_FD(s)` and `INVALID_SOCKET_HANDLE`
keep their contracts.

---

## Raw Ethernet HAL Interface (Step 2+)

`socket_raw.c` depends on a minimal HAL that the application or platform port
must provide.  The HAL is defined in `socket_raw_hal.h` (to be created in
Step 2):

```c
// Send one raw Ethernet frame. Returns bytes sent or -1 on error.
int16_t eth_hal_send(const uint8_t *frame, uint16_t len);

// Receive one raw Ethernet frame (blocking, with timeout).
// Returns bytes received, 0 on timeout, -1 on error.
int16_t eth_hal_recv(uint8_t *frame, uint16_t max_len, uint32_t timeout_ms);

// Get the local MAC address.
void eth_hal_get_mac(uint8_t mac[6]);
```

On Linux the HAL implementation uses `AF_PACKET` (see Step 3).  On bare-metal
it wraps the target's EMAC driver.

---

## Step Plan

### Step 1 — Stubs (`src/socket_raw.c`) ✓ done

- Define `#ifdef OPTION_ENABLE_UDP_RAW` guard.
- Implement all 10 API functions as stubs: `socketStartup` returns `false`,
  send/recv return `-1`, others are no-ops.
- Add `socket_raw.c` to `xcplite_SOURCES` in `CMakeLists.txt`.
- Add `OPTION_ENABLE_UDP_RAW` variant to `sockets.h` outer guard and to the
  description comment.
- Exclude `sockets.c` from compilation when `OPTION_ENABLE_UDP_RAW` is set.
- Activate in `src/xcplib_rtos_cfg.h` (`#undef OPTION_ENABLE_UDP`, `#define OPTION_ENABLE_UDP_RAW`).

Verified: stubs compile and link cleanly in the `rtos` configuration:

```bash
cmake -B build-rtos -S . -DXCPLITE_CONFIGURATION=rtos -DXCPLITE_BUILD_EXAMPLES=ON
cmake --build build-rtos
```

### Step 2 — Minimal UDP/IP layer

- Introduce `struct socket_raw` as `SOCKET_HANDLE` type.
- Implement `socketOpen`: allocate struct, open raw Ethernet HAL.
- Implement `socketBind`: store local IP/port; send a gratuitous ARP request
  (broadcast, announces local IP→MAC mapping).
- Implement `socketSendTo`: build Ethernet frame (dst MAC from `dst_mac`,
  EtherType 0x0800), IPv4 header (TTL=64, proto=17/UDP, checksum), UDP header,
  copy payload, call `eth_hal_send`.
- Implement `socketRecvFrom`: call `eth_hal_recv`, validate EtherType/IP/UDP
  headers, extract source IP/port, return payload.
- Implement ARP receive path inside `socketRecvFrom`: if EtherType == 0x0806
  and ARP is a Reply for our IP, capture sender MAC into `dst_mac`.
- IP/UDP checksum: compute using RFC 768 / RFC 791 algorithms.
- No fragmentation; MTU assumed ≥ `XCPTL_MAX_SEGMENT_SIZE` + headers (≤ 1500 bytes).

### Step 3 — Linux integration test

- Create `socket_raw_hal_linux.c`: implements the HAL using
  `socket(AF_PACKET, SOCK_RAW, htons(ETH_P_ALL))`.
  Requires `CAP_NET_RAW` (`sudo` or `setcap cap_net_raw+ep`).
- Add a test configuration (e.g. `socket_raw_linux`) to `build.sh` /
  `CMakeLists.txt`.
- Run `no_a2l_demo` or `hello_xcp` against this transport and connect with
  CANape or `xcpclient`.
- Validate: connect, upload, download, DAQ measurement.

### Step 4 — Zero-copy queue optimization

**Goal:** avoid copying the XCP payload into a separate send buffer before
prepending the Ethernet/IP/UDP header.

**Design:**  
Reserve `SOCKET_RAW_HEADER_RESERVE` bytes (= 14 + 20 + 8 = 42 bytes for
Ethernet + IPv4 + UDP) at the front of every queue entry in `queue32` and
`queue32m`.  On transmit, `socketSendTo` writes the headers directly into those
reserved bytes and calls `eth_hal_send` with a pointer to the start of the
reservation — no memcpy of the payload.

Changes required:
- New compile-time constant `XCPTL_TRANSPORT_LAYER_HEADER_RESERVE` (default 0,
  set to 42 when `OPTION_ENABLE_UDP_RAW`).
- `queue32.c` / `queue32m.c`: add `XCPTL_TRANSPORT_LAYER_HEADER_RESERVE` bytes
  before each entry's `size` field; adjust all pointer arithmetic.
- `xcpethtl.c`: pass the raw-entry pointer to `socketSendTo`; the function
  writes headers into `ptr - XCPTL_TRANSPORT_LAYER_HEADER_RESERVE`.
- `socketSendTo` signature gains an optional `header_reserve` in-out
  parameter, or the offset is baked in via the option.

This optimization is **only** for `queue32` / `queue32m`.  The 64-bit
lock-free queues (`queue64v`, `queue64f`) are not changed; `OPTION_ENABLE_UDP_RAW`
implies a 32-bit / embedded target where those queues are not used.

---

## ARP Design (Step 2)

Minimal ARP sufficient for point-to-point XCP (target ↔ one PC):

1. **Gratuitous ARP** on `socketBind`: broadcast ARP Announcement
   (sender = target IP/MAC, target = target IP/MAC).  This populates the
   PC's ARP cache so it can reach the target immediately.
2. **ARP reply parsing** in `socketRecvFrom`: when an ARP Reply arrives with
   `target_ip == src_ip`, store `sender_mac` into `socket->dst_mac` and set
   `peer_known = true`.
3. **ARP request response**: when an ARP Request arrives asking for our IP,
   send an ARP Reply.  This handles cases where the PC flushes its ARP cache.
4. **Unicast fallback**: until `peer_known`, outgoing frames use broadcast
   destination MAC `ff:ff:ff:ff:ff:ff`.  After the first ARP exchange,
   unicast MAC is used.

No ARP cache table is needed; one fixed peer is tracked per socket.

---

## Future Backends

The HAL interface (`eth_hal_send` / `eth_hal_recv` / `eth_hal_get_mac`) is
designed to be thin enough to wrap:

- **Vector XLAPI** (Windows): `xlCanTransmit`-equivalent for Ethernet channels;
  `socket_raw_hal_xlapi.c`.
- **ASAM CMP**: CMP wraps XCP frames in a CMP header over Ethernet;
  `socket_raw_hal_cmp.c` would add/strip the CMP envelope and route to the
  correct channel.

Both backends share the same `socket_raw.c` UDP/IP layer; only the HAL file
differs.



## TODO

- SHM does not compile without OPTION_ENABLE_TCP
- RAW is not intended to be used with SHM, but the build system still tries to compile it.

---

## Current Code State (after Step 1)

### Files created
| File | Purpose |
|---|---|
| `src/socket_raw.c` | Stub implementations, all wrapped in `#ifdef OPTION_ENABLE_UDP_RAW` |
| `docs/SOCKET_RAW.md` | This document |

### Files modified
| File | Change |
|---|---|
| `src/sockets.h` | Outer guard extended to `\|\| defined(OPTION_ENABLE_UDP_RAW)`; description comment updated |
| `src/sockets.c` | Outer guard changed to `&& !defined(OPTION_ENABLE_UDP_RAW)` so the normal socket implementation compiles away when raw is selected |
| `CMakeLists.txt` | `src/socket_raw.c` added to `xcplite_SOURCES` |

### How OPTION_ENABLE_UDP_RAW is activated

Set in `src/xcplib_rtos_cfg.h`:

```c
#undef  OPTION_ENABLE_TCP
#undef  OPTION_ENABLE_UDP
#define OPTION_ENABLE_UDP_RAW
```

The stubs in `socket_raw.c` are therefore active and linked in the `rtos`
configuration build.  The build was verified clean:

```bash
cmake -B build-rtos -S . -DXCPLITE_CONFIGURATION=rtos -DXCPLITE_BUILD_EXAMPLES=ON
cmake --build build-rtos
```

### SOCKET_HANDLE type (current)

For the rtos/POSIX-simulator path, `SOCKET_HANDLE` is still the plain `int` fd
type defined by the `#if !defined(_WIN) && !defined(OPTION_SOCKET_HW_TIMESTAMPS)`
branch in `sockets.h`.  Step 2 will replace this with `struct socket_raw *`
under an `#elif defined(OPTION_ENABLE_UDP_RAW)` branch in `sockets.h`.

### Codebase context

`socket_raw.c` was introduced as part of a larger refactor that split
`platform.c/.h` into:
- `platform.c/.h` — threads, mutex, clock, sleep, memory, atomics
- `sockets.c/.h` — socket abstraction for all standard platforms

`socket_raw.c` is the third peer alongside `sockets.c`, activated exclusively
by `OPTION_ENABLE_UDP_RAW`.


