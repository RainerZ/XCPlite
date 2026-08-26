# OPTION_ENABLE_UDP_RAW — Raw-Ethernet XCP/UDP Transport

## Motivation

XCPlite supports XCP-on-Ethernet through the OS socket API (`OPTION_ENABLE_UDP` /
`OPTION_ENABLE_TCP`). Some embedded targets have no TCP/IP stack at all and expose
only a raw Ethernet send/receive interface — a direct EMAC driver, or an RTOS
Ethernet abstraction without lwIP. This build variant implements the XCP UDP/IPv4
transport entirely inside XCPlite, directly on top of raw Ethernet frames.

Two further backends motivate the HAL abstraction:

- **Vector XLAPI** (Windows) — raw frame access on Vector VN interface hardware
- **ASAM CMP** (Capture Module Protocol) — for testing XCP tools which communicate
  through capture modules

---

## Compilation guard

`OPTION_ENABLE_UDP_RAW` is **mutually exclusive** with `OPTION_ENABLE_UDP` and
`OPTION_ENABLE_TCP`, and **requires `OPTION_QUEUE_32`**. Both are enforced by
`#error` in `src/xcptl_cfg.h`:

| Guard | Reason |
|---|---|
| not with `OPTION_ENABLE_UDP` / `OPTION_ENABLE_TCP` | exactly one transport per build |
| requires `OPTION_QUEUE_32` | the 64 bit queues transmit with `socketSendToV` (scatter-gather), which the raw transport does not implement. Without this guard a 64 bit build fails at link time with no hint about the cause |
| not with `OPTION_SHM_MODE` | SHM needs `queueInitFromMemory`, which exists only in `queue64v.c` / `queue64f.c` |
| not with `XCPTL_ENABLE_MULTICAST` | `socketJoin` is not provided |

### MTU and frame size

There is deliberately **no compile-time MTU guard**. The link MTU is a runtime property that only
the target knows, so hard-coding a limit would bake a "standard Ethernet" assumption into
`xcptl_cfg.h` and would wrongly forbid a jumbo-capable link.

Note what `OPTION_MTU` means: it is the link MTU rounded up to a multiple of 8, and the 14 byte
**Ethernet header is not part of it**. `XCPTL_MAX_SEGMENT_SIZE = OPTION_MTU - 32` reserves 28 bytes
for the IPv4 and UDP headers plus the 4 bytes of that round-up (1500 -> 1504), so the resulting IP
packet is `OPTION_MTU - 4` bytes. The invariant is `OPTION_MTU <= link MTU + 4`.

An `OPTION_MTU` too large for the link is reported at runtime, and both transports behave the same
way because **neither fragments IPv4**:

| Transport | Mechanism |
|---|---|
| socket (UDP) | `socketOpen` sets DF (`IP_PMTUDISC_DO` / `IP_DONTFRAG` / `IP_DONTFRAGMENT`), so `sendto` fails with `EMSGSIZE` |
| raw Ethernet | `eth_hal_send` returns `ETH_HAL_ERROR_SIZE`, mapped to `SOCKET_ERROR_MSGSIZE` |

Both print the segment size and the `OPTION_MTU` to reduce; the raw HAL additionally names the
interface and its MTU, since only the backend knows that. Observed on a link forced to MTU 1000:

```
ERROR: eth_hal_send: frame of 1242 bytes is too large for interface veth1 (MTU 1000, so at most 1014 bytes per frame)
ERROR: socketSendTo: segment of 1200 bytes does not fit into one Ethernet frame on this link.
  Reduce OPTION_MTU (currently 1504, giving XCPTL_MAX_SEGMENT_SIZE=1472), see the interface MTU reported above.
```

The transport also asserts a **little endian host** (`src/socket_raw.c`) and the
availability of a **HAL backend** (`src/socket_raw_hal.h`). Only the Linux AF_PACKET
backend exists today, so a macOS or Windows build of the `raw` configuration stops
with a clear message rather than an obscure link error.

---

## Build configuration

RAW has its own configuration, `XCPLITE_CONFIGURATION=raw` → `src/xcplib_raw_cfg.h`,
building into `build-raw/`:

```bash
./build.sh raw examples      # library + udp_raw_demo   (Linux only)
./build.sh raw tests         # library + socket_raw_test (Linux only)
```

It is deliberately **not** an override inside the `rtos` configuration: `rtos` targets
use the lwIP socket API (`OPTION_FREERTOS_LWIP`) or host sockets in the POSIX
simulator, and both must keep working. An embedded target without an IP stack defines
`OPTION_ENABLE_UDP_RAW` in its own external build (PlatformIO, CubeMX) and supplies its
own HAL backend.

Options in `src/xcplib_raw_cfg.h`:

| Option | Default | Purpose |
|---|---|---|
| `OPTION_UDP_RAW_IFNAME` | `"eth0"` | default interface, overridden by `socketRawSetInterface()` |
| `OPTION_UDP_RAW_ENABLE_ICMP_ECHO` | on | answer ping — the single most useful bring-up aid |
| `OPTION_UDP_RAW_UDP_CHECKSUM_ZERO` | on | transmit UDP checksum 0, legal for IPv4 (RFC 768) |
| `OPTION_UDP_RAW_UDP_CHECKSUM_COMPUTE` | off | RFC 768 software checksum |
| `OPTION_UDP_RAW_UDP_CHECKSUM_HW` | off | leave 0, the EMAC inserts it |
| `OPTION_UDP_RAW_VERIFY_RX_CHECKSUM` | on | verify received IPv4 header checksums |
| `OPTION_UDP_RAW_GRATUITOUS_ARP` | off | announce our IP/MAC on bind |
| `OPTION_UDP_RAW_ZERO_COPY` | off | reserve header space in the queue (see *Zero copy*, not implemented yet) |

Note on the zero UDP checksum default: `tcpdump` and Wireshark can then not validate the
UDP framing. Switch to `_COMPUTE` temporarily while bringing up a new target.

---

## API subset

Only these functions from `sockets.h` exist in a RAW build; the rest are removed from
the header, so reaching for one is a compile error rather than a link error:

| Function | Notes |
|---|---|
| `socketStartup` / `socketCleanup` | initialize / release the transport |
| `socketGetErrorString` / `socketGetLastError` | self contained error codes, no `<errno.h>` |
| `socketOpen` | opens the Ethernet HAL, reads the local MAC. UDP only, rejects `SOCKET_MODE_TCP` |
| `socketBind` | stores the local IP and UDP port. Rejects `0.0.0.0` |
| `socketRecvFrom` | receives one UDP datagram, answers ARP and ICMP on the way |
| `socketSendTo` | builds Ethernet/IPv4/UDP headers and transmits |
| `socketSetTimeout` | receive timeout, RX only |
| `socketShutdown` / `socketClose` | unblock a receive / release the HAL |
| `socketRawSetInterface` | RAW only: select the interface before `XcpEthServerInit()` |
| `socketRawGetLocalMac` | RAW only: local MAC, used for the A2L `IF_DATA` |

Not provided: TCP (`socketListen`/`socketAccept`/`socketRecv`/`socketSend`), multicast
(`socketJoin`), scatter-gather (`socketSendToV`/`socketSendV`), hardware timestamps,
`socketGetMAC`, `socketGetLocalAddr`.

---

## Architecture

```
xcpethtl.c / xcpethserver.c
        │  sockets.h API (subset above)
        ▼
  socket_raw.c
    ├── UDP/IPv4 layer   (header build and parse, checksums)
    ├── ARP              (answer requests for our IP)
    ├── ICMP             (answer Echo Requests)
    └── receive filter + deadline loop
        │  socket_raw_hal.h
        ▼
  ┌──────────────────────────────────────────────────┐
  │ socket_raw_hal_linux.c   AF_PACKET  (implemented) │
  │ socket_raw_hal_xlapi.c   Vector XLAPI   (future)  │
  │ socket_raw_hal_cmp.c     ASAM CMP       (future)  │
  └──────────────────────────────────────────────────┘
```

### Local address

There is no IP stack and no DHCP, so `socketBind(0.0.0.0)` has no meaning. The
**application supplies the IPv4 address** through the `address` parameter of
`XcpEthServerInit()`, which already exists; `socketBind` rejects all-zero, broadcast,
multicast and loopback addresses with an explanatory error. The **MAC comes from the
HAL** (`eth_hal_get_mac`) — every EMAC and every AF_PACKET interface knows its own.

Both values are also stored in `gXcpTl.server_addr` / `server_mac`, so `XcpEthTlGetInfo`
and the A2L `IF_DATA` report the real address without needing
`OPTION_ENABLE_GET_LOCAL_ADDR`.

### ARP: answer only

XCP is always master initiated — the tool sends `CONNECT` first — so the peer MAC, IP
and UDP port all arrive with that first frame. Consequently:

1. **ARP Requests for our IP are answered.** This is mandatory: the IP stack of the XCP
   client resolves us before it can send anything.
2. **The peer is learned from the accepted UDP datagram**, never from ARP. An unrelated
   host asking for our IP must not be able to redirect the DAQ stream.
3. **No ARP Requests are ever sent**, and ARP Replies are ignored.
4. A gratuitous ARP announcement on bind is available but off by default.

A consequence worth knowing: because the peer MAC is learned rather than resolved,
**no netmask and no default gateway are needed**. With a client behind a router, the
router MAC arrives as the frame source and the responses go back to it.

### ICMP Echo

Answering ping is enabled by default. It is the highest value bring-up milestone: a
successful `ping <target>` proves the Ethernet HAL, the MAC filter, the ARP responder,
the IPv4 header build and the header checksum all work, before any XCP tooling is
involved.

### Receive: filter and deadline loop

A raw socket sees every frame on the wire. `socketRecvFrom` therefore **loops
internally against an absolute deadline** computed once on entry, rather than returning
0 for every foreign frame — otherwise the caller would run its full background task
suite once per foreign frame, and that cadence would depend on link load instead of on
`XCPTL_RECV_TIMEOUT_MS`.

Filter order, cheapest and most discriminating first:

1. EtherType — ARP goes to the responder, VLAN (`0x8100`) is dropped with a warning so a
   trunk port is diagnosable, anything else is dropped
2. destination MAC — ours or broadcast
3. IPv4 sanity, **fragments rejected with a warning** (there is no reassembly),
   destination IP, optional header checksum verification
4. protocol — ICMP goes to the responder
5. destination UDP port — where almost every remaining frame on a busy link dies
6. payload size — **dropped, never truncated**: a truncated message would surface as a
   confusing "Corrupt message received!" from the transport layer

Return contract, identical to `sockets.c`: `> 0` bytes, `== 0` timeout (the caller does
background work and loops), `< 0` closed or error (the caller exits its loop).
`socketShutdown` sets a flag and wakes a blocked receive through the HAL.

### Transmit serialization

ARP and ICMP replies are generated in the **receive** thread, while command responses
and DAQ segments are transmitted from their own paths, so `eth_hal_send` is called from
two threads. `struct socket_raw` therefore owns a `tx_mutex` which covers header
construction **and** the HAL call, keeping the HAL contract simple: `eth_hal_send` does
not need to be reentrant.

This mutex is deliberately **independent of `gXcpTl.ctr_mutex`**. That one happens to
serialize transmissions today, but it exists for a different reason — XCP requires the
message counter to increase monotonically across command responses and DAQ messages —
and removing it is a future goal. Nothing in `socket_raw.c` depends on it.

---

## Raw Ethernet HAL

`src/socket_raw_hal.h` — a port has to provide send and receive of complete Ethernet
frames plus the local MAC, nothing else:

```c
bool     eth_hal_open(const char *config, tEthHalCtx **ctx);
void     eth_hal_close(tEthHalCtx *ctx);
bool     eth_hal_get_mac(tEthHalCtx *ctx, uint8_t *mac);
int16_t  eth_hal_send(tEthHalCtx *ctx, const uint8_t *frame, uint16_t len);
int16_t  eth_hal_recv(tEthHalCtx *ctx, uint8_t *frame, uint16_t max_len, uint32_t timeout_ms);
void     eth_hal_wakeup(tEthHalCtx *ctx);   // optional, may be a no-op
```

`config` is backend specific and opaque to `socket_raw.c`: the interface name on Linux,
an application/channel selector for XLAPI, a device and stream id for CMP.

**There is no per-frame channel parameter.** Every foreseeable backend would pass a
constant, the backends do not share a channel type (XLAPI channel index vs. CMP device
id + stream id + interface id vs. nothing at all for AF_PACKET), and a CMP concept must
not leak into the core. Backend identity is configuration, not a per-frame value.

Frames are complete Ethernet frames **without FCS**. Frames as short as 50 bytes are
passed; a port whose MAC does not pad to the 60 byte Ethernet minimum must do it itself.

### Linux backend (`socket_raw_hal_linux.c`)

`AF_PACKET`/`SOCK_RAW`/`ETH_P_ALL` bound to one interface. Needs `CAP_NET_RAW`:

```bash
sudo setcap cap_net_raw+ep ./build-raw/udp_raw_demo
```

Two details that matter: `PACKET_IGNORE_OUTGOING` (plus a `PACKET_OUTGOING` check as the
portable fallback) stops our own transmitted frames from coming straight back into the
receive path; and a blocked receive is unblocked with an `eventfd` and `poll()` rather
than `SO_RCVTIMEO`, so shutdown is immediate and an infinite timeout stays interruptible.

### ASAM CMP

CMP is a **HAL backend, fully hidden**. It exists to test XCP tools which communicate
through capture modules, not as a feature for ECU developers, so nothing in the core is
optimized or parameterized for it:

- the CMP backend receives a complete Ethernet/IPv4/UDP frame and applies its envelope
  **into its own buffer**, inside the HAL
- it does not borrow the queue headroom and does not participate in the zero copy path
- device id, stream id and interface selection are parsed from the `config` string

The extra copy is accepted — this is a test bench path. If a CMP driven change ever
appears to be needed in `socket_raw.c`, the queue or the config headers, that is the
signal that the encapsulation has leaked; fix it in the HAL.

---

## Testing

See `test/test_socket_raw.sh`. Development happens on Linux (a Raspberry Pi over ssh).

### Phase A — isolated, one machine

The kernel IP stack sees every frame on an interface, so if the target IP were a host
address the kernel would answer the ARP itself and send ICMP port unreachable for the XCP
UDP port. A network namespace avoids that. `lo` cannot be used — it is `ARPHRD_LOOPBACK`
and has no Ethernet header.

```bash
./build.sh raw examples
sudo ./test/test_socket_raw.sh          # ARP, ping and XCP CONNECT checks
sudo ./test/test_socket_raw.sh --keep   # leave it running for manual tests
```

The script creates a veth pair with the target in namespace `xcpraw` and **no kernel IP
on the target side**, so `socket_raw.c` alone owns `192.168.90.2`. This phase needs only
`ping`, `arping` and `tcpdump` — no XCP tooling has to be built on the target machine.

### Phase B — real LAN

```bash
sudo setcap cap_net_raw+ep ./build-raw/udp_raw_demo
./build-raw/udp_raw_demo --if eth0 --ip 192.168.1.240   # spare, outside the DHCP pool
```

The address must be outside the DHCP pool and not otherwise in use. The kernel does not
own it, so it drops the datagrams at the IP layer while AF_PACKET still hands us a copy
at the link layer — which is why this works without a namespace. Drive it with
`xcpclient` and CANape from another machine. Expect `PACKET_IGNORE_OUTGOING` to matter
much more here, and the receive filter to be exercised by real background traffic.

### Bring-up order

1. `ping <target>` — HAL, MAC filter, ARP responder, IPv4 header and checksum, in one shot
2. `arping -I veth0 <target>` — isolates ARP from IP
3. `tcpdump -i veth0 -nn -e -vv` alongside everything: it prints `bad ip cksum` explicitly.
   Build with `OPTION_UDP_RAW_UDP_CHECKSUM_COMPUTE` for this step so the UDP checksum can
   be validated too. Confirm full segments are exactly 1514 bytes
4. `xcpclient` CONNECT / GET_STATUS — source address and port extraction, peer MAC
   learning, and the `socketSendTo` return value contract
5. UPLOAD / DOWNLOAD — larger command responses
6. DAQ measurement, long enough to wrap the transmit queue ring
7. Shutdown — the receive thread must exit within ~100 ms, and the idle receive loop must
   not burn CPU (watch `top`; that is the symptom of a broken deadline loop)
8. Adversarial receive: `ping -s 3000` (fragments dropped, not parsed), UDP to the wrong
   port, broadcast UDP, and an oversized datagram (dropped, not truncated)

### Unit tests

`test/socket_raw_test` covers everything that does not need a network — checksums against
the RFC 1071 reference vector, wire struct packing, the frame build, the receive filter
and the ARP and ICMP responders — by compiling `socket_raw.c` with a fake HAL that
captures the transmitted frame:

```bash
./build.sh raw tests && ./build-raw/socket_raw_test
```

---

## Not implemented yet

### Zero copy transmit (`OPTION_UDP_RAW_ZERO_COPY`)

`socketSendTo` currently copies the payload into a frame buffer to prepend the 42 byte
header. The plan is to reserve headroom in front of every transmit queue segment so the
header can be written in place.

This is far cheaper than it looks: a `queue32`/`queue32m` entry is a whole segment
(`tXcpSegmentBuffer.msg_buffer[XCPTL_MAX_SEGMENT_SIZE]`), so it is **one added field**
in each of the two files and no pointer arithmetic changes — every access is
`msg_buffer` relative, and `queuePop` already returns `&msg_buffer`. With a 48 byte
reservation `msg_buffer` stays 8 byte aligned and the IPv4 header lands 4 byte aligned.

Two things it needs: the command response path builds its message on the stack
(`XcpTlSendCrm`) and has no headroom, so it keeps the copying `socketSendTo` while the
DAQ segment path gets a separate in-place entry point; and the 64 bit queues are not
changed, which the `OPTION_QUEUE_32` guard already enforces.

Deferred until the transport is validated on hardware.

### Command path latency

`GET_DAQ_CLOCK` is used for time synchronization, so jitter in handling it degrades
synchronization quality. The current design optimizes for throughput, not latency. Known
raw specific jitter sources, for a later optimization pass:

1. **inline ARP/ICMP replies** — an ARP burst or ping flood delays the command behind it.
   Largest controllable source; could be deferred or rate limited
2. **filter work per foreign frame** — small, but scales with link load
3. **`tx_mutex` contention** — a command response can wait behind a DAQ segment send.
   Should be short, but on AF_PACKET `eth_hal_send` is a `write()` syscall, so measure

A further optimization would remove the header build from the lock entirely: almost the
whole 42 byte header is constant per client and could be prepared once on connect, with
only `total_length`, the UDP length and the IPv4 checksum patched per send. Fixing the
IPv4 `ident` at 0 (legal for atomic datagrams with DF set, RFC 6864) removes the last
shared mutable state. Note the lengths do vary: `queuePop` flushes partial segments.

If the jitter turns out to be unsatisfactory, the better answer is not a transport
optimization at all — XCP allows telling the client that the `GET_DAQ_CLOCK` timestamp is
sampled close to **response transmission** rather than command reception, which makes the
receive path jitter largely irrelevant. That is not implemented today.

### Other

- VLAN (802.1Q) is out of scope; tagged frames are dropped with a warning
- Vector XLAPI and ASAM CMP backends
