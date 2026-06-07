#pragma once

#include <lwipopts_default.h>

#undef NO_SYS
#define NO_SYS 0

#undef SYS_LIGHTWEIGHT_PROT
#define SYS_LIGHTWEIGHT_PROT 1

#undef LWIP_NETCONN
#define LWIP_NETCONN 1

#undef LWIP_SOCKET
#define LWIP_SOCKET 1
