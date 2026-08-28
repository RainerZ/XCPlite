# cmp_demo — XCP over ASAM CMP, with an out-of-tree Ethernet HAL

Demonstrates supplying **your own backend** for the xcplib raw Ethernet transport
(`OPTION_ENABLE_UDP_RAW`) from outside the library. xcplite is used as installed and
unmodified: it builds plain Ethernet/IPv4/UDP frames and hands them to the six `eth_hal_*`
functions, which this project implements.

ASAM CMP (Capture Module Protocol) serves **testing of XCP tools that communicate through
capture modules**. It is not an ECU developer feature, so nothing about it lives in
libxcplite — that separation is the point of this example.

> **Status: the CMP envelope is not implemented yet.** `src/cmp.c` is a pass-through, so the
> demo currently behaves exactly like the plain raw Ethernet transport. That is deliberate:
> it makes the out-of-tree HAL plumbing testable on its own, so a later failure is
> unambiguously in the envelope and not in the wiring.

> **Linux only.** The backend uses `AF_PACKET` and needs `CAP_NET_RAW`.

---

## How the override works

`libxcplite`'s `socket_raw.c` calls six `eth_hal_*` functions. This project defines all six in
`src/socket_raw_hal_cmp.c`. Because **libxcplite is a static library**, the linker only pulls an
archive member in to resolve an *undefined* symbol — and these are already defined here, so the
built-in AF_PACKET backend (`socket_raw_hal_linux.o`) is never pulled in and there is no clash.

Verified on the target: `nm` shows `eth_hal_open` resolved from this project, and the built-in
backend's strings are absent from the binary.

Two caveats worth knowing:

- **It depends on xcplite being a static library.** A shared build would resolve `eth_hal_*`
  internally and the override would not take.
- **On macOS or Windows** the library additionally has to be built with
  `OPTION_UDP_RAW_HAL_EXTERNAL`, because there is no built-in backend on those platforms and the
  raw configuration would not compile at all. On Linux that option is not needed.

### Files

| File | Purpose |
|---|---|
| `src/main.c` | Demo application — command line, server setup, calibration segment, event |
| `src/socket_raw_hal_cmp.c` | The Ethernet HAL backend: AF_PACKET frame I/O, calls the envelope |
| `src/cmp.h` / `src/cmp.c` | The CMP envelope — **the only place that knows about CMP** |

---

## Verified against

| | |
|---|---|
| xcplite | https://github.com/RainerZ/XCPlite, `raw` configuration |
| Library version | 2.1.2 (as reported by `find_package`) |
| Target | Raspberry Pi 5, Debian, aarch64, GCC |
| Checked | builds from outside the xcplite tree; `nm` confirms the built in AF_PACKET backend is not linked in; `ping` and XCP CONNECT answered through this backend |

This project only needs an **installed** xcplite, not a source checkout. If you move it into a
repository of its own, that install is the whole dependency.

---

## Building

xcplite has to be installed from the `raw` configuration first:

```bash
cd <xcplite>
cmake -B build-raw -S . -DXCPLITE_CONFIGURATION=raw -DCMAKE_BUILD_TYPE=Debug \
      -DCMAKE_INSTALL_PREFIX=$HOME/xcplite-install
cmake --build build-raw --target install
```

Then build this project against it:

```bash
cd examples/cmp_demo
cmake -B build -S . -Dxcplite_DIR=$HOME/xcplite-install/lib/cmake/xcplite
cmake --build build
```

---

## Running

```bash
sudo setcap cap_net_raw+ep ./build/cmp_demo
./build/cmp_demo --if eth0 --ip 192.168.0.221
```

The address must be a spare one that the kernel does not own — xcplib answers ARP and ICMP for it
itself. See [udp_raw_demo](https://github.com/RainerZ/XCPlite/blob/master/examples/udp_raw_demo/README.md) for the isolated
`veth`/netns setup, which works here unchanged while the envelope is a pass-through.

```bash
ping 192.168.0.221                 # ARP, ICMP and the IPv4 header, all through our own HAL
xcpclient --dest-addr 192.168.0.221:5555 --udp --upload-a2l --a2l ./cmp_demo.a2l
xcpclient --dest-addr 192.168.0.221:5555 --udp --a2l ./cmp_demo.a2l --list-mea . --list-cal .
```

`xcpclient` does not upload the A2L automatically — without `--upload-a2l --a2l <file>` it expects
a file named after the target's `GET_ID ASAM_NAME` in the current directory. The uploaded A2L also
starts with `/include "XCP_104.aml"`, so copy that file next to it (it ships with xcplite) or the
parse fails.

---

## Implementing the CMP envelope

Everything CMP-specific goes in `src/cmp.c`, behind `cmpWrap()` / `cmpUnwrap()`. Set
`CMP_ENVELOPE_IMPLEMENTED` to 1 there once they do something.

**Settle these from the specification before writing code** — they are recorded as `@@@@ TODO`
in `src/cmp.c`:

1. **Injection.** CMP is primarily a capture protocol, module → host. XCP needs
   request/response, so the capture module must also be able to inject towards the ECU. If it
   cannot, a CMP backend could only carry DAQ and XCP could not even CONNECT — that would
   reshape this demo fundamentally. **Answer this first.**
2. **The header layout** — version, device id, message type, stream id, sequence counter, and
   for data messages the interface id, timestamp and payload length. Take these from the
   specification rather than guessing; `CMP_ETHERTYPE` in `cmp.h` is likewise marked for
   verification.
3. **What the payload is** — the complete Ethernet frame, as the pass-through structure
   assumes, or the IP packet without the Ethernet header.
4. **The sequence counter** — who increments it, and whether the receiver must check it.
5. **The capture timestamp.** `eth_hal_recv` has no timestamp parameter today, so surfacing
   CMP's would need an interface extension in xcplib's `socket_raw_hal.h`. Cheaper to decide
   now than once several backends exist.

Two constraints that come from the xcplib side and should not be worked around:

- The envelope is applied **into the backend's own buffer**, never into the transmit queue
  headroom. CMP must not participate in the zero-copy path or influence `XCPTL_TX_HEADROOM`.
- The extra copy that costs is accepted — this is a test-bench path, not a performance one.

If a CMP-driven change ever appears to be needed inside libxcplite, that is the signal that the
encapsulation has leaked; fix it in the backend instead.

See [docs/SOCKET_RAW.md](https://github.com/RainerZ/XCPlite/blob/master/docs/SOCKET_RAW.md) for the transport design and
the HAL contract.
