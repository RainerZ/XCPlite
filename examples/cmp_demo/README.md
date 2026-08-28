# cmp_demo — XCP tunnelled through an emulated ASAM CMP capture module

Demonstrates supplying **your own backend** for the xcplib raw Ethernet transport
(`OPTION_ENABLE_UDP_RAW`) from outside the library. xcplite is used as installed and
unmodified: it builds plain Ethernet/IPv4/UDP frames and hands them to the six `eth_hal_*`
functions, which this project implements.

ASAM CMP (Capture Module Protocol) serves **testing of XCP tools that communicate through
capture modules**. It is not an ECU developer feature, so nothing about it lives in
libxcplite — that separation is the point of this example.

Implements **ASAM CMP 1.1.0**: Ethernet Data Message payloads (`0x08`) over UDP (§6.4.2), in
both directions, plus the read-only part of the REST interface.

---

## What it does

The demo emulates a capture module with one interface, behind which sits one XCP ECU. The
tool never addresses the ECU over IP — everything is tunnelled:

```
   XCP tool  ============ CMP over UDP ============  cmp_demo
  (Data Sink)                                    (Capture Module)
                                                        |
      <--- CAP_DATA_MSG (0x01) -- captured frames -------+--- emulated ECU
      ---- TX_DATA_MSG  (0x04) -- injected frames ------>+    (xcplib)
```

Transmission — `TX_DATA_MSG`, message type `0x04` — is what CMP **1.1** added, and it is what
makes XCP possible at all here: without it a capture module could only carry DAQ one way and
XCP could never even CONNECT.

| Direction | What happens |
|---|---|
| ECU → tool | The frame xcplib hands to `eth_hal_send` is what the capture module just captured, so it is wrapped as a Captured Data Message and sent to the Data Sink |
| tool → ECU | A Transmit Data Message is unwrapped and its inner frame handed to `eth_hal_recv`; xcplib parses it as ordinary Ethernet/IPv4/UDP and answers the XCP command inside |

### Wire format

All CMP fields are big endian (§6.2). Sizes are fixed by the specification.

```
CMP header (8 B, §6.2.1)          version=1, reserved, DeviceId, MessageType,
                                  StreamId, StreamSequenceCounter
Captured Data Message hdr (16 B)  Timestamp, InterfaceId, CommonFlags,
                                  PayloadType=0x08, PayloadLength           §7.2.1
Transmit Data Message hdr (24 B)  Timestamp, Deadline, InterfaceId,
                                  TransmissionOptions, CommonFlags,
                                  PayloadType=0x08, PayloadLength           §7.2.2
Ethernet payload (6 B + data)     Flags, Reserved, DataLength, DATA         §7.3.8
```

`DATA` runs from the destination MAC **through the FCS**, but xcplib's HAL contract passes
frames *without* FCS — so the codec appends four zero bytes on wrap and strips four on
unwrap. `FCS_SUPPORT` is reported as 0, which is what §7.3.8 prescribes for a module that
cannot compute a real FCS. Payload type `0x0D RAW_ETHERNET` is deliberately not used: it
additionally carries the preamble and SFD, which a synthetic ECU has nothing useful to put in.

Not used, and advertised as unsupported over REST: aggregation (§6.3.2), segmentation
(§6.3.3), status messages (§8) and control messages (§9).

---

## How the override works

`libxcplite`'s `socket_raw.c` calls six `eth_hal_*` functions. This project defines all six in
`src/socket_raw_hal_cmp.c`. Because **libxcplite is a static library**, the linker only pulls an
archive member in to resolve an *undefined* symbol — and these are already defined here, so a
built-in backend is never pulled in and there is no clash.

Two caveats worth knowing:

- **It depends on xcplite being a static library.** A shared build would resolve `eth_hal_*`
  internally and the override would not take.
- **On macOS or Windows** the library additionally has to be built with
  `OPTION_UDP_RAW_HAL_EXTERNAL`, because there is no built-in backend on those platforms and the
  raw configuration would not compile at all. On Linux that option is not needed.

### Files

| File | Purpose |
|---|---|
| `src/cmp.h` / `src/cmp.c` | The CMP envelope codec — **the only place that knows about CMP**. Pure: no sockets, no I/O, no global state |
| `src/cmp_transport.h` | The outer transport seam |
| `src/cmp_transport_udp.c` | CMP over UDP (§6.4.2) — one ordinary datagram socket |
| `src/socket_raw_hal_cmp.c` | The six `eth_hal_*` functions, joining codec and transport |
| `src/cmp_backend.h` | Backend configuration and status |
| `src/cmp_rest.h` / `src/cmp_rest.c` | The read-only REST interface (§12.3) |
| `src/main.c` | Demo application — command line, server setup, calibration segment, event |
| `test/cmp_codec_test.c` | Codec unit test against the specification's own sample files |
| `test/fake_sink.py` | A minimal Data Sink — plays the tool's half |
| `test/test.sh` | End-to-end test |

---

## Building

xcplite has to be installed from the `raw` configuration first:

```bash
cd <xcplite>
cmake -B build-raw -S . -DXCPLITE_CONFIGURATION=raw -DCMAKE_BUILD_TYPE=Debug \
      -DCMAKE_INSTALL_PREFIX=$HOME/xcplite-install
cmake --build build-raw --target install
```

On macOS or Windows add `-DCMAKE_C_FLAGS="-DOPTION_UDP_RAW_HAL_EXTERNAL"` to that first
command. Then build this project against the install:

```bash
cd examples/cmp_demo
cmake -B build -S . -Dxcplite_DIR=$HOME/xcplite-install/lib/cmake/xcplite
cmake --build build
```

This project only needs an **installed** xcplite, not a source checkout. If you move it into a
repository of its own, that install is the whole dependency.

---

## Running

```bash
./build/cmp_demo --sink 192.168.0.10:55555
```

**No privileges are needed** — the CMP transport is an ordinary UDP socket, not `AF_PACKET`.
`--sink` may be omitted, in which case the Data Sink address is learned from the first CMP
message received, the same way `socket_raw.c` learns the peer MAC.

| Option | Meaning |
|---|---|
| `--sink <a.b.c.d:port>` | Data Sink address for captured data. Default: learned |
| `--listen <port>` | UDP port for incoming CMP messages (default 55555) |
| `--mtu <bytes>` | MTU of the path to the Data Sink (default 1500, max 9000) |
| `--rest-port <port>` | REST interface port (default 8080, 0 disables) |
| `--device-id`, `--stream-id`, `--interface-id` | CMP identity |
| `--ecu-mac <xx:…:xx>` | MAC of the emulated ECU. Default: derived from `--device-id` |
| `--ip`, `--port` | Address of the emulated ECU, seen only inside the CMP payload |

Because the ECU address only ever appears inside the payload, it does **not** have to be free
on any real network — unlike the plain raw Ethernet transport. `ping` to it does not work
either: there is no IP route to the ECU, it lives behind the tunnel. Send an ICMP echo as a
`TX_DATA_MSG` instead if you want to exercise that path.

---

## The MTU constraint

§6.4.2: *"CMP messages shall not be sent over IP fragmented packets."* The envelope adds 34
bytes to every captured frame, and the outer IPv4/UDP headers add 28 more, so on a 1500-byte
path the largest inner frame is **1438 bytes**.

xcplite's `raw` configuration uses `OPTION_MTU 1504`, which produces frames of up to 1514
bytes. Small transfers are unaffected, but a saturated DAQ stream will hit the limit and
report `SOCKET_ERROR_MSGSIZE`. The demo reports the budget and the remedy at startup:

```
  CMP frame budget: 1438 bytes per inner frame (1472 byte CMP message - 34 byte envelope)
WARNING: xcplib can produce frames of up to 1514 bytes, but only 1438 fit into one
         un-fragmented CMP message on this path. ...
```

Two ways out:

- **Raise the outer MTU.** `--mtu 9000` on a jumbo-capable path; §6.4 allows this explicitly.
- **Reduce `OPTION_MTU`.** `OPTION_MTU 1424` in `src/xcplib_raw_cfg.h` fits a 1500-byte path
  exactly (segment 1392, inner frame 1434, IP packet 1496). This is a **library build-config
  decision**, not a code change, and it means the demo no longer consumes a stock `raw` install
  — so it is left open rather than made silently.

Either way, an oversized frame is refused with `ETH_HAL_ERROR_SIZE` rather than fragmented.
That is exactly what the HAL contract designed that error for: whether a frame fits is a
runtime property only the backend knows.

---

## The REST interface

§12.3 calls the REST interface mandatory for a capture module, and §7.2.2 says a Data Sink
uses it **to detect whether transmission is supported**. If a tool gates injection on that,
no REST means no XCP — which is why the read-only part is implemented here.

| Endpoint | § |
|---|---|
| `GET /asam-cmp/version-info` | 12.3.1 |
| `GET /asam-cmp/v1/identification` | 12.3.2 |
| `GET /asam-cmp/v1/interfaces` | 12.3.4 — **advertises transmission support** |
| `GET /asam-cmp/v1/measurement` | 12.3.6 |

The decisive part is the `Transmitter` object of `/interfaces` (§12.3.4, Table 88):

```json
"Transmitter": { "TransmissionSupportBitmask": 1, "FeatureSupportBitmask": 0,
                 "AggregationMtu": 1472, "AggregationCount": 1 }
```

`TransmissionSupportBitmask` bit 0 is `TIMESTAMP_IMMEDIATE`: we send every request straight
away and support neither absolute nor relative scheduling, no deadline and no segmentation.
§7.2.2 allows that — *"If the CM does not support Timestamp, it shall always send
immediately"*. `AggregationMtu` is where the MTU budget above is advertised to the tool.

§12.3 says the interface *should* run on port 80; the demo defaults to 8080 so it needs no
privileges. Everything that would change configuration (the `PUT` methods), mDNS/DNS-SD
discovery (§12.2.2) and XCP-based discovery (§12.1) are **not** implemented — §12 permits
*"Static configuration without Capture Module Discovery"*, which is what this demo uses.

---

## Testing

```bash
./test/test.sh
```

Runs the codec unit test, then starts `cmp_demo` and drives it with `fake_sink.py`. No `veth`
pair and no network namespace are needed, unlike the plain raw Ethernet transport: the outer
transport is an ordinary UDP socket and loopback is enough. Nothing needs root.

**`test/cmp_codec_test.c`** is the strongest check. Its golden vectors are lifted byte for byte
from the sample PCAPNG files shipped with the specification, so it pins the wire format against
the standard itself rather than against one reading of it:

- `CMP_1.0/asam_cmp_cap_0x08_Ethernet.pcapng` — a Captured Data Message with an Ethernet
  payload, exactly the shape this backend emits. `cmpWrapCaptured()` must reproduce it byte for
  byte (with the sample's FCS zeroed, since we report `FCS_SUPPORT = 0`).
- `CMP_1.1/asam_cmp_tx_0x01_can_29bit_0x12345678.pcapng` — a real Transmit Data Message. Its
  payload is CAN, so it must be *rejected* — but only after the 24-byte transmit header has
  been parsed correctly. A wrong header length surfaces as `MALFORMED` instead of
  `PAYLOAD_TYPE`, so this vector pins the transmit header layout too.

There is no sample of a transmit message carrying an Ethernet payload — the 1.1 samples cover
CAN, CAN FD and LIN only — so the happy path uses a message built from those two pinned
layouts.

**`test/fake_sink.py`** plays the tool's half with nothing but the Python standard library: it
queries the REST endpoints, checks that transmission is advertised, and tunnels XCP CONNECT /
GET_STATUS / DISCONNECT, verifying the responses and the `StreamSequenceCounter` continuity.
`--pcap <file>` writes every CMP message to a capture file for **Wireshark**, whose built-in
ASAM CMP dissector keys on EtherType `0x99FE` — so the messages are framed for the Ethernet
transport option there and dissect automatically. The CMP bytes are identical under both
transport options; only the outer framing differs.

The [openDAQ ASAM-CMP-Library](https://github.com/openDAQ/ASAM-CMP-Library) can serve as an
independent decoder for the capture direction, but not for transmission: its `MessageType` enum
stops at `0x03`, so it predates CMP 1.1 and does not know `TX_DATA_MSG`.

---

## Verified against

| | |
|---|---|
| xcplite | https://github.com/RainerZ/XCPlite, `raw` configuration |
| Library version | 2.1.2 (as reported by `find_package`) |
| Specification | ASAM CMP Protocol Layer Specification V1.1.0, 2026-01-31 |
| Host | macOS 15 / arm64 / Apple clang, library built with `OPTION_UDP_RAW_HAL_EXTERNAL` |
| Checked | `nm` confirms all six `eth_hal_*` are undefined in `libxcplite.a`; codec test 74/74 against the specification's sample files; XCP CONNECT, GET_STATUS and DISCONNECT tunnelled end to end through CMP; REST endpoints answered; emitted messages re-decoded from the pcap |
| **Not** yet checked | Linux and the Raspberry Pi 5 target; Wireshark dissection (no Wireshark on the verification host); DAQ under load, which is where the MTU limit bites; CANape |

---

## Not implemented

- **CMP over Ethernet, EtherType 0x99FE (§6.4.1).** This is the transport option the
  specification makes **mandatory** for a capture module, so it should not be deferred
  indefinitely. It needs the AF_PACKET plumbing — preserved in git commit `01e7f40`, which used
  it for the pass-through version of this backend — plus an outer Ethernet header
  (dst = sink MAC, src = our MAC, EtherType `0x99FE`) and padding to the 60-byte minimum.
  The envelope codec itself is unchanged: only `cmp_transport.h` gains an implementation.
- Status messages (§8), control messages (§9), aggregation, segmentation, time
  synchronisation and the configuration-changing REST methods.

---

## Constraints that should not be worked around

- Nothing CMP-specific belongs in libxcplite. If a change there appears necessary, that is the
  signal that the encapsulation has leaked — fix it in the backend. **No such change was
  needed:** `socket_raw_hal.h` is untouched, including for the capture timestamp, which the
  backend takes itself in `eth_hal_send` because that is the moment the emulated module sees
  the frame.
- The envelope is applied **into the backend's own buffer**, never into the transmit queue
  headroom. CMP must not participate in the zero-copy path or influence `XCPTL_TX_HEADROOM`.
- The extra copy that costs is accepted — this is a test-bench path, not a performance one.

See [docs/SOCKET_RAW.md](https://github.com/RainerZ/XCPlite/blob/master/docs/SOCKET_RAW.md) for the transport design and
the HAL contract.
