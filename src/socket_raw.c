/*----------------------------------------------------------------------------
| File:
|   socket_raw.c
|
| Description:
|   Raw-Ethernet XCP/UDP transport (OPTION_ENABLE_UDP_RAW)
|   Step 1: stubs — see docs/SOCKET_RAW.md for the full implementation plan
|
| Code released into public domain, no attribution required
 ----------------------------------------------------------------------------*/

#include "sockets.h"

#include "assert.h"
#include "dbg_print.h"

#ifdef OPTION_ENABLE_UDP_RAW

const char *socketGetErrorString(int32_t err) {
    (void)err;
    return "raw socket error";
}

bool socketStartup(void) {
    DBG_PRINT_ERROR("socket_raw: not implemented\n");
    return false;
}

void socketCleanup(void) {}

bool socketOpen(SOCKET_HANDLE *socketp, uint16_t flags) {
    (void)flags;
    assert(socketp != NULL);
    *socketp = INVALID_SOCKET_HANDLE;
    DBG_PRINT_ERROR("socket_raw: socketOpen not implemented\n");
    return false;
}

bool socketBind(SOCKET_HANDLE socket, const uint8_t *addr, uint16_t port) {
    (void)socket;
    (void)addr;
    (void)port;
    DBG_PRINT_ERROR("socket_raw: socketBind not implemented\n");
    return false;
}

bool socketShutdown(SOCKET_HANDLE socket) {
    (void)socket;
    return true;
}

bool socketClose(SOCKET_HANDLE *socketp) {
    assert(socketp != NULL);
    *socketp = INVALID_SOCKET_HANDLE;
    return true;
}

int16_t socketRecvFrom(SOCKET_HANDLE socket, uint8_t *buffer, uint16_t bufferSize, uint8_t *srcAddr, uint16_t *srcPort, uint64_t *time) {
    (void)socket;
    (void)buffer;
    (void)bufferSize;
    (void)srcAddr;
    (void)srcPort;
    (void)time;
    DBG_PRINT_ERROR("socket_raw: socketRecvFrom not implemented\n");
    return -1;
}

int16_t socketSendTo(SOCKET_HANDLE socket, const uint8_t *buffer, uint16_t bufferSize, const uint8_t *addr, uint16_t port, uint64_t *time) {
    (void)socket;
    (void)buffer;
    (void)bufferSize;
    (void)addr;
    (void)port;
    (void)time;
    DBG_PRINT_ERROR("socket_raw: socketSendTo not implemented\n");
    return -1;
}

bool socketSetTimeout(SOCKET_HANDLE socket, uint32_t timeoutMs) {
    (void)socket;
    (void)timeoutMs;
    return true;
}

bool socketGetMAC(char *ifname, uint8_t *mac) {
    (void)ifname;
    (void)mac;
    DBG_PRINT_ERROR("socket_raw: socketGetMAC not implemented\n");
    return false;
}

#endif // OPTION_ENABLE_UDP_RAW
