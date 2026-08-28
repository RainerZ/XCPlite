#pragma once

/*----------------------------------------------------------------------------
| File:
|   cmp.h
|
| Description:
|   ASAM CMP (Capture Module Protocol) envelope for the cmp_demo Ethernet HAL backend.
|
|   This is the ONLY place in the demo that knows about CMP. socket_raw_hal_cmp.c
|   handles raw Ethernet frame I/O and calls cmpWrap() before sending and cmpUnwrap()
|   after receiving. xcplib itself knows nothing about CMP - it builds and parses plain
|   Ethernet/IPv4/UDP frames and hands them to the HAL. See docs/SOCKET_RAW.md.
|
|   STATUS: the envelope is NOT implemented yet. cmpWrap()/cmpUnwrap() are pass through,
|   so the demo currently behaves exactly like the plain raw Ethernet transport. That
|   makes the out of tree HAL plumbing testable on its own - ping and XCP CONNECT work -
|   before any CMP specific code exists. See the README for what has to be filled in.
|
| Code released into public domain, no attribution required
 ----------------------------------------------------------------------------*/

#include <stdbool.h>
#include <stdint.h>

// EtherType assigned to ASAM CMP.
// @@@@ TODO: verify against the ASAM CMP specification before relying on it.
#define CMP_ETHERTYPE 0x99FE

// Configuration of the local capture module, see cmpInit()
typedef struct {
    uint16_t device_id; // identifies this capture module
    uint8_t stream_id;  // stream within the device
} tCmpConfig;

// Initialize the envelope layer
void cmpInit(const tCmpConfig *config);

// Wrap one complete Ethernet frame for transmission.
// in/in_len:   the frame built by xcplib (Ethernet + IPv4 + UDP + XCP payload)
// out/out_max: buffer for the wrapped frame, owned by the caller
// Returns the wrapped length, or 0 if it does not fit.
//
// Note the envelope is applied into the CALLER's buffer, never into the transmit queue
// headroom: CMP must not participate in the zero copy path and must not influence
// XCPTL_TX_HEADROOM. The extra copy is accepted, this is a test bench path.
uint16_t cmpWrap(const uint8_t *in, uint16_t in_len, uint8_t *out, uint16_t out_max);

// Unwrap one received frame.
// Returns the length of the extracted inner Ethernet frame, or 0 if this frame is not a
// CMP data message addressed to us and should be discarded.
uint16_t cmpUnwrap(const uint8_t *in, uint16_t in_len, uint8_t *out, uint16_t out_max);

// True while the envelope is a pass through, i.e. no CMP header is added or expected.
// The HAL uses this to decide whether it may talk plain Ethernet.
bool cmpIsPassThrough(void);
