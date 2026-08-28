/*----------------------------------------------------------------------------
| File:
|   socket_raw_hal_cmp.c
|
| Description:
|   Ethernet HAL backend for the cmp_demo, implementing src/socket_raw_hal.h of xcplib.
|
|   This file lives in the demo, NOT in the library: CMP serves testing of XCP tools
|   through capture modules, it is not an ECU developer feature, so nothing about it
|   belongs in libxcplite. The library is used as installed and unmodified. Because
|   libxcplite is a static library and this object defines all six eth_hal_* symbols,
|   the linker never pulls the built in AF_PACKET backend out of the archive.
|
|   Structure:
|     eth_hal_send/recv here   raw Ethernet frame I/O over AF_PACKET
|     cmp.c                    the CMP envelope (currently a pass through)
|
|   The AF_PACKET plumbing deliberately duplicates src/socket_raw_hal_linux.c rather than
|   sharing it: keeping the demo self contained is what keeps CMP out of the library.
|
| Code released into public domain, no attribution required
 ----------------------------------------------------------------------------*/

#include <arpa/inet.h>       // for htons
#include <errno.h>           // for errno
#include <linux/if_ether.h>  // for ETH_P_ALL
#include <linux/if_packet.h> // for sockaddr_ll, PACKET_OUTGOING, PACKET_IGNORE_OUTGOING
#include <net/if.h>          // for ifreq, IFNAMSIZ
#include <poll.h>            // for poll
#include <stdio.h>           // for printf
#include <stdlib.h>          // for malloc, free
#include <string.h>          // for memset, memcpy, strncpy, strerror
#include <sys/eventfd.h>     // for eventfd
#include <sys/ioctl.h>       // for ioctl, SIOCGIFINDEX, SIOCGIFHWADDR
#include <sys/socket.h>      // for socket, bind, setsockopt, recvfrom
#include <sys/types.h>       // for ssize_t
#include <unistd.h>          // for close, read, write

#include <assert.h>

#include "cmp.h"
#include "socket_raw_hal.h" // the interface this file implements, installed with xcplib

#define ETH_HDR_LEN 14 // Ethernet header, not covered by the interface MTU
#define MAX_FRAME 2048 // enough for one Ethernet frame plus a CMP envelope

struct eth_hal_ctx {
    int fd;                // AF_PACKET socket
    int wakeup_fd;         // eventfd used by eth_hal_wakeup()
    int ifindex;           // interface index
    unsigned int mtu;      // interface MTU, the largest IP packet, excluding the Ethernet header
    uint8_t mac[6];        // interface MAC address
    char ifname[IFNAMSIZ]; // interface name
    uint8_t tx[MAX_FRAME]; // wrapped frame, built by cmpWrap
    uint8_t rx[MAX_FRAME]; // frame as received, before cmpUnwrap
};

bool eth_hal_open(const char *config, tEthHalCtx **ctxp) {

    assert(ctxp != NULL);
    *ctxp = NULL;

    const char *ifname = (config != NULL && config[0] != 0) ? config : "eth0";

    tEthHalCtx *ctx = (tEthHalCtx *)malloc(sizeof(tEthHalCtx));
    if (ctx == NULL) {
        printf("ERROR: eth_hal_open: out of memory\n");
        return false;
    }
    memset(ctx, 0, sizeof(tEthHalCtx));
    ctx->fd = -1;
    ctx->wakeup_fd = -1;
    strncpy(ctx->ifname, ifname, sizeof(ctx->ifname) - 1);

    // ETH_P_ALL, not ETH_P_IP: ARP frames must be seen as well, otherwise the XCP client
    // cannot resolve our MAC and the target is unreachable.
    ctx->fd = socket(AF_PACKET, SOCK_RAW, htons(ETH_P_ALL));
    if (ctx->fd < 0) {
        if (errno == EPERM || errno == EACCES) {
            printf("ERROR: eth_hal_open: AF_PACKET socket denied (errno=%d, %s).\n"
                   "  The raw Ethernet transport needs CAP_NET_RAW:\n"
                   "    sudo setcap cap_net_raw+ep <binary>\n",
                   errno, strerror(errno));
        } else {
            printf("ERROR: eth_hal_open: AF_PACKET socket failed (errno=%d, %s)\n", errno, strerror(errno));
        }
        goto error;
    }

    struct ifreq ifr;
    memset(&ifr, 0, sizeof(ifr));
    strncpy(ifr.ifr_name, ctx->ifname, IFNAMSIZ - 1);
    if (ioctl(ctx->fd, SIOCGIFINDEX, &ifr) < 0) {
        printf("ERROR: eth_hal_open: interface '%s' not found (errno=%d, %s)\n", ctx->ifname, errno, strerror(errno));
        goto error;
    }
    ctx->ifindex = ifr.ifr_ifindex;

    memset(&ifr, 0, sizeof(ifr));
    strncpy(ifr.ifr_name, ctx->ifname, IFNAMSIZ - 1);
    if (ioctl(ctx->fd, SIOCGIFHWADDR, &ifr) < 0) {
        printf("ERROR: eth_hal_open: SIOCGIFHWADDR for '%s' failed (errno=%d, %s)\n", ctx->ifname, errno, strerror(errno));
        goto error;
    }
    memcpy(ctx->mac, ifr.ifr_hwaddr.sa_data, 6);

    memset(&ifr, 0, sizeof(ifr));
    strncpy(ifr.ifr_name, ctx->ifname, IFNAMSIZ - 1);
    if (ioctl(ctx->fd, SIOCGIFMTU, &ifr) < 0) {
        ctx->mtu = 1500; // only used for diagnostics
    } else {
        ctx->mtu = (unsigned int)ifr.ifr_mtu;
    }

    struct sockaddr_ll sll;
    memset(&sll, 0, sizeof(sll));
    sll.sll_family = AF_PACKET;
    sll.sll_protocol = htons(ETH_P_ALL);
    sll.sll_ifindex = ctx->ifindex;
    if (bind(ctx->fd, (struct sockaddr *)&sll, sizeof(sll)) < 0) {
        printf("ERROR: eth_hal_open: bind to '%s' failed (errno=%d, %s)\n", ctx->ifname, errno, strerror(errno));
        goto error;
    }

    // Do not loop our own transmitted frames back into the receive path
#ifdef PACKET_IGNORE_OUTGOING
    int one = 1;
    if (setsockopt(ctx->fd, SOL_PACKET, PACKET_IGNORE_OUTGOING, &one, sizeof(one)) < 0) {
        printf("WARNING: eth_hal_open: PACKET_IGNORE_OUTGOING not available, using the sll_pkttype filter only\n");
    }
#endif

    ctx->wakeup_fd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    if (ctx->wakeup_fd < 0) {
        printf("ERROR: eth_hal_open: eventfd failed (errno=%d, %s)\n", errno, strerror(errno));
        goto error;
    }

    printf("  CMP Ethernet HAL on %s (index %d), MAC=%02X:%02X:%02X:%02X:%02X:%02X, MTU=%u\n", ctx->ifname, ctx->ifindex, ctx->mac[0], ctx->mac[1], ctx->mac[2], ctx->mac[3],
           ctx->mac[4], ctx->mac[5], ctx->mtu);
    if (cmpIsPassThrough()) {
        printf("  CMP envelope: PASS THROUGH (not implemented yet), behaves like plain raw Ethernet\n");
    }

    *ctxp = ctx;
    return true;

error:
    if (ctx->fd >= 0)
        close(ctx->fd);
    if (ctx->wakeup_fd >= 0)
        close(ctx->wakeup_fd);
    free(ctx);
    return false;
}

void eth_hal_close(tEthHalCtx *ctx) {
    if (ctx == NULL)
        return;
    if (ctx->fd >= 0)
        close(ctx->fd);
    if (ctx->wakeup_fd >= 0)
        close(ctx->wakeup_fd);
    free(ctx);
}

bool eth_hal_get_mac(tEthHalCtx *ctx, uint8_t *mac) {
    assert(ctx != NULL && mac != NULL);
    // A CMP capture module has no MAC of its own in the same sense. This value is only used
    // as the Ethernet source of the frame xcplib builds, which CMP then wraps, so the
    // interface MAC is a reasonable choice - a synthetic locally administered MAC would do.
    memcpy(mac, ctx->mac, 6);
    return true;
}

int16_t eth_hal_send(tEthHalCtx *ctx, const uint8_t *frame, uint16_t len) {

    assert(ctx != NULL && frame != NULL);

    // Apply the CMP envelope into our own buffer. Never into the transmit queue headroom:
    // CMP must not participate in the zero copy path (see cmp.h).
    uint16_t tx_len = cmpWrap(frame, len, ctx->tx, (uint16_t)sizeof(ctx->tx));
    if (tx_len == 0) {
        printf("ERROR: eth_hal_send: frame of %u bytes does not fit the CMP envelope buffer\n", len);
        return ETH_HAL_ERROR_SIZE;
    }

    for (;;) {
        ssize_t n = write(ctx->fd, ctx->tx, tx_len);
        if (n < 0) {
            if (errno == EINTR)
                continue;
            if (errno == EMSGSIZE) {
                printf("ERROR: eth_hal_send: frame of %u bytes is too large for interface %s (MTU %u, so at most %u bytes per frame)\n", tx_len, ctx->ifname, ctx->mtu,
                       ctx->mtu + ETH_HDR_LEN);
                return ETH_HAL_ERROR_SIZE;
            }
            printf("ERROR: eth_hal_send: write failed (errno=%d, %s)\n", errno, strerror(errno));
            return ETH_HAL_ERROR;
        }
        if (n != (ssize_t)tx_len) {
            printf("ERROR: eth_hal_send: partial write %zd of %u bytes\n", n, tx_len);
            return ETH_HAL_ERROR;
        }
        // Report the length xcplib handed us, not the wrapped length: the envelope is invisible above
        return (int16_t)len;
    }
}

int16_t eth_hal_recv(tEthHalCtx *ctx, uint8_t *frame, uint16_t max_len, uint32_t timeout_ms) {

    assert(ctx != NULL && frame != NULL);

    struct pollfd pfd[2];
    pfd[0].fd = ctx->fd;
    pfd[0].events = POLLIN;
    pfd[0].revents = 0;
    pfd[1].fd = ctx->wakeup_fd;
    pfd[1].events = POLLIN;
    pfd[1].revents = 0;

    int r = poll(pfd, 2, (int)timeout_ms);
    if (r < 0) {
        if (errno == EINTR)
            return 0;
        printf("ERROR: eth_hal_recv: poll failed (errno=%d, %s)\n", errno, strerror(errno));
        return ETH_HAL_ERROR;
    }
    if (r == 0)
        return 0; // timeout

    if (pfd[1].revents & POLLIN) { // wakeup requested, drain and report no frame
        uint64_t v;
        ssize_t rd = read(ctx->wakeup_fd, &v, sizeof(v));
        (void)rd;
        return 0;
    }
    if (!(pfd[0].revents & POLLIN))
        return 0;

    struct sockaddr_ll from;
    socklen_t fromlen = sizeof(from);
    memset(&from, 0, sizeof(from));
    ssize_t n = recvfrom(ctx->fd, ctx->rx, sizeof(ctx->rx), 0, (struct sockaddr *)&from, &fromlen);
    if (n < 0) {
        if (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)
            return 0;
        printf("ERROR: eth_hal_recv: recvfrom failed (errno=%d, %s)\n", errno, strerror(errno));
        return ETH_HAL_ERROR;
    }

    // Portable fallback for PACKET_IGNORE_OUTGOING: drop our own transmitted frames
    if (from.sll_pkttype == PACKET_OUTGOING)
        return 0;

    // Strip the CMP envelope. 0 means this frame is not for us, the caller keeps waiting.
    uint16_t inner = cmpUnwrap(ctx->rx, (uint16_t)n, frame, max_len);
    return (int16_t)inner;
}

void eth_hal_wakeup(tEthHalCtx *ctx) {
    if (ctx == NULL || ctx->wakeup_fd < 0)
        return;
    uint64_t v = 1;
    ssize_t nw = write(ctx->wakeup_fd, &v, sizeof(v));
    (void)nw;
}
