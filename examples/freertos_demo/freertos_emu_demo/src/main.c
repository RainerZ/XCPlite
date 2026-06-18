// freertos_demo - XCPlite example for FreeRTOS POSIX simulator
//
// Demonstrates XCPlite (XCP measurement and calibration) running inside FreeRTOS tasks.
// Uses the FreeRTOS POSIX simulator port so the demo builds and runs on macOS / Linux
// before switching to the real embedded target.
//
// Relationship to no_a2l_demo:
//   - Same measurement/calibration variables and XCP server setup without on-target A2L generation
//   - Threads are replaced by FreeRTOS tasks (xTaskCreate / vTaskDelayUntil)
//   - Timing uses pdMS_TO_TICKS() / vTaskDelayUntil() instead of sleepUs()
//   - The xcplite library itself still uses POSIX sockets and POSIX threads internally;
//     these coexist with FreeRTOS tasks without interference in the POSIX simulator.
//
// How the POSIX simulator works:
//   Each FreeRTOS task runs as a separate pthread managed by the FreeRTOS scheduler.
//   The scheduler uses SIGUSR1/SIGUSR2 sent to specific threads (not the whole process),
//   so normal POSIX APIs (sockets, clocks) work as expected inside FreeRTOS tasks.
//
// No A2L generation:
//   The demo does not generate an A2L file at runtime.
//   Use the pre-supplied freertos_demo.a2l or generate one with the tool xcpclient
//
// Build:
//   cmake -B build -S . -DXCPLITE_BUILD_FREERTOS_DEMO=ON
//   cmake --build build --target freertos_demo
//
// Connect with CANape or any XCP tool to UDP port 5555.

#include <assert.h>  // for assert (used by DaqTriggerEvent macro)
#include <signal.h>  // for signal, SIGINT, SIGTERM
#include <stdbool.h> // for bool
#include <stdint.h>  // for uintxx_t
#include <stdio.h>   // for printf
#include <stdlib.h>  // for malloc, free
#include <string.h>  // for memset

// FreeRTOS kernel headers
#include "FreeRTOS.h"
#include "semphr.h"
#include "task.h"

// XCPlite headers
#include "platform.h" // for platform abstraction (sleepUs, get_thread_id, ...)
#include "xcplib.h"   // for libxcplite application programming interface

// Demo application header
#include "../xcp_demo.h"

//-----------------------------------------------------------------------------
// Signal handler – clean shutdown on Ctrl-C or SIGTERM
//
// The FreeRTOS POSIX port explicitly keeps SIGINT unblocked in all task
// threads (port.c: sigdelset(&xAllSignals, SIGINT)) so the handler is always
// reachable. vTaskEndScheduler() leaves the calling task's pthread blocked
// in event_wait() forever, so we exit directly instead.

static void sig_handler(int sig) {
    (void)sig;

    printf("\nShutting down XCP server...\n");
    XcpDisconnect();
    XcpEthServerShutdown();

    printf("exit\n");
    exit(0);
}

//-----------------------------------------------------------------------------
// main

int main(int argc, char *argv[]) {

    signal(SIGINT, sig_handler);
    signal(SIGTERM, sig_handler);

    printf("XCPlite FreeRTOS POSIX Simulator Demo\n");

    // Initialise XCPlite *before* the FreeRTOS scheduler starts.
    xcp_demo_init();

    // Start the FreeRTOS scheduler (blocks until vTaskEndScheduler() is called)
    printf("Starting FreeRTOS scheduler...\n");
    vTaskStartScheduler(); // Never returns

    return 0;
}
