// fetchcontent_example - C version demonstrating libxcplite built from source via FetchContent

// Disclaimer:
// This example does not use calibration segments, so thread safety is not guaranteed for direct write access to global calibration variables.

#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

// Include libxcplite headers from the installed location
#include <a2l.h>
#include <xcplib.h>

//-----------------------------------------------------------------------------------------------------
// XCP configuration

#define OPTION_PROJECT_NAME "fetchcontent_example"
#define OPTION_PROJECT_VERSION "V100"
#define OPTION_USE_TCP true
#define OPTION_SERVER_PORT 5555
#define OPTION_SERVER_ADDR {0, 0, 0, 0}
#define OPTION_QUEUE_SIZE (1024 * 32)
#define OPTION_LOG_LEVEL 4

//-----------------------------------------------------------------------------------------------------

// Simple calibration parameter
uint32_t loop_delay_us = 1000;

// Simple measurement variable
uint32_t counter = 0;

//-----------------------------------------------------------------------------------------------------
// Signal handling

static volatile bool g_appRunning = true;

static void signalHandler(int sig) {
    (void)sig;
    printf("\nShutdown signal received\n");
    g_appRunning = false;
}

//-----------------------------------------------------------------------------------------------------
// Main

int main(void) {

    // Install signal handlers
    signal(SIGINT, signalHandler);
    signal(SIGTERM, signalHandler);

    // Set XCP log level
    XcpSetLogLevel(OPTION_LOG_LEVEL);

    // Initialize XCP
    XcpInit(OPTION_PROJECT_NAME, OPTION_PROJECT_VERSION, XCP_MODE_PERSISTENCE | XCP_MODE_LOCAL);

    // Initialize XCP Ethernet server
    uint8_t addr[4] = OPTION_SERVER_ADDR;
    if (!XcpEthServerInit(addr, OPTION_SERVER_PORT, OPTION_USE_TCP, OPTION_QUEUE_SIZE)) {
        printf("ERROR: XCP initialization failed\n");
        return 1;
    }

    printf("XCP server listening on %s port %d\n", OPTION_USE_TCP ? "TCP" : "UDP", OPTION_SERVER_PORT);
    printf("Connect CANape to this address to start measurement\n\n");

    // Enable A2L generation
    if (!A2lInit(addr, OPTION_SERVER_PORT, OPTION_USE_TCP, A2L_MODE_WRITE_ONCE | A2L_MODE_FINALIZE_ON_CONNECT)) {
        return 1;
    }

    // Create a measurement event and a global measurement variable
    DaqCreateEvent(MainTask);
    A2lSetAbsoluteAddrMode(MainTask);
    A2lCreateMeasurement(counter, "Incrementing counter");

    // Create a global calibration parameter (not using a calibration segment, thread safety not guaranteed)
    A2lCreateParameter(loop_delay_us, "Loop delay in microseconds", "us", 100, 100000);

    printf("Starting main loop (press Ctrl+C to stop)...\n\n");

    // Main application loop
    while (g_appRunning) {

        // Increment counter
        counter++;

        // Trigger XCP measurement event
        DaqTriggerEvent(MainTask);

        // Sleep using calibration value
        usleep(loop_delay_us);
    }

    XcpDisconnect();        // Force disconnect the XCP client
    A2lFinalize();          // Finalize A2L generation, if not done yet
    XcpEthServerShutdown(); // Stop the XCP server
    return 0;
}
