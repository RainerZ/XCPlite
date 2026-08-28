/*----------------------------------------------------------------------------
| File:
|   cmp.c
|
| Description:
|   ASAM CMP envelope - NOT IMPLEMENTED YET, currently a pass through.
|   See cmp.h for the role of this file and the README for what has to be filled in.
|
| Code released into public domain, no attribution required
 ----------------------------------------------------------------------------*/

#include "cmp.h"

#include <string.h>

// Set to 1 once cmpWrap/cmpUnwrap actually implement the CMP envelope
#define CMP_ENVELOPE_IMPLEMENTED 0

static tCmpConfig sConfig = {.device_id = 0, .stream_id = 0};

void cmpInit(const tCmpConfig *config) {
    if (config != NULL) {
        sConfig = *config;
    }
}

bool cmpIsPassThrough(void) { return CMP_ENVELOPE_IMPLEMENTED == 0; }

#if CMP_ENVELOPE_IMPLEMENTED

// @@@@ TODO: implement the ASAM CMP envelope here.
//
// What has to be decided from the specification before writing this:
//
//  1. Injection. CMP is primarily a capture protocol, capture module -> host. XCP needs
//     request/response, so the module must also be able to inject towards the ECU. If it
//     cannot, a CMP HAL backend can only carry DAQ and XCP could not even CONNECT, which
//     would change the design of this demo fundamentally. Settle this first.
//  2. The header layout: version, device id, message type, stream id, sequence counter,
//     and for data messages the interface id, timestamp and payload length. Do not guess
//     these - take them from the specification.
//  3. Whether the payload is the complete Ethernet frame (as assumed by the pass through
//     structure here) or the IP packet without the Ethernet header.
//  4. The sequence counter: who increments it and whether the receiver has to check it.
//  5. Whether the capture timestamp should be surfaced. eth_hal_recv has no timestamp
//     parameter today, so using it would need an interface extension in socket_raw_hal.h -
//     cheaper to decide now than after several backends exist.

#else

// Pass through: no CMP header is added or expected. The demo then behaves exactly like the
// plain raw Ethernet transport, which is what makes the out of tree HAL plumbing testable
// on its own before any CMP specific code exists.

uint16_t cmpWrap(const uint8_t *in, uint16_t in_len, uint8_t *out, uint16_t out_max) {
    (void)sConfig;
    if (in_len > out_max) {
        return 0;
    }
    memcpy(out, in, in_len);
    return in_len;
}

uint16_t cmpUnwrap(const uint8_t *in, uint16_t in_len, uint8_t *out, uint16_t out_max) {
    if (in_len > out_max) {
        return 0;
    }
    memcpy(out, in, in_len);
    return in_len;
}

#endif // CMP_ENVELOPE_IMPLEMENTED
