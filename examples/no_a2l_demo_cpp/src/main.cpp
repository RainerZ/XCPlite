// no_a2l_demo_cpp XCPlite example
// Demonstrates XCPlite operation without runtime A2L generation
// See ../README.md for details
// Requires manual or tool based XCPlite specific A2L file creation and update process

#include <array>    // for std::array
#include <atomic>   // for std::atomic
#include <csignal>  // for signal handling
#include <cstdint>  // for uintxx_t
#include <iostream> // for std::cout
#include <optional> // for std::optional

#include "xcplib.hpp" // for libxcplite application programming interface

// Internal libxcplite includes to simplify multi platform support
#include "platform.h" // for platform abstraction - thread local, threads, mutex, sockets, sleepUs, ...

// Signal handler for graceful exit on Ctrl+C
std::atomic<bool> gRun{true};
void signal_handler(int signal) {
    if (signal == SIGINT || signal == SIGTERM) {
        gRun = false;
    }
}

//-----------------------------------------------------------------------------------------------------
// XCP parameters

constexpr const char OPTION_PROJECT_NAME[] = "no_a2l_demo_cpp"; // Project name, used to build the A2L and BIN file name
constexpr const char OPTION_PROJECT_VERSION[] = "109";          // EPK version string
constexpr bool OPTION_USE_TCP = false;                          // TCP or UDP
constexpr uint8_t OPTION_SERVER_ADDR[] = {0, 0, 0, 0};          // Bind addr, 0.0.0.0 = ANY
constexpr uint16_t OPTION_SERVER_PORT = 5555;                   // Port
constexpr uint16_t OPTION_QUEUE_SIZE = (1024 * 32);             // Size of the queue in bytes, should be large enough to cover at least 10ms of expected traffic
constexpr int OPTION_LOG_LEVEL = 3;                             // Log level, 0 = no log, 1 = error, 2 = warning, 3 = info, 4 = debug

//-----------------------------------------------------------------------------------------------------
// Demo calibration parameters

// Global calibration parameters
struct params {

    uint32_t delay_us; // Sleep time in microseconds for the main and the task loop

    // Calibration parameters of various basic and complex types
    double test_par_double;
    bool test_par_bool;
    uint64_t test_par_uint64;
    uint32_t test_par_uint32;
    uint16_t test_par_uint16;
    enum { OFF = 0, ON = 1, STANDBY = 2 } test_par_enum;
    uint8_t test_par_uint8_array[10];
    struct test_par_struct {
        uint16_t test_field_uint16;
        int16_t test_field_int16;
        float test_field_float;
        uint8_t test_field_uint8_array[3];
    } test_par_struct;
};
const struct params kParams = {.delay_us = 1000,
                               .test_par_double = 0.123456789,
                               .test_par_bool = true,
                               .test_par_uint64 = 0x1234567812345678,
                               .test_par_uint32 = 0x1234,
                               .test_par_uint16 = 0x1234,
                               .test_par_enum = params::ON,
                               .test_par_uint8_array = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9},
                               .test_par_struct = {2, -2, 0.4f, {0, 1, 2}}};

// Define a global calibration parameter segment named 'params' for the calibration parameters in 'const struct params kParams' as default/reference page
// @@@@ CalSegDecl(params);
std::optional<xcplib::CalSeg<struct params>> gCalSeg;

// Example metadata annotations For the calibration parameter 'delay_us' in the params calibration segment

// Metadata annotation as comments for Vector A2L Toolset Creator
/*
@@ STRUCTURE = params
@@ ELEMENT = delay_us
@@ DESCRIPTION = "Sleep time in microseconds for the main and the task loop"
@@ UNIT = "us"
@@ LIMITS = 1.0, 10000.0
@@ DATA_TYPE = UWORD [0 ... 10000]
@@ END
*/

// Metadata annotation as code (static data in a special ELF section)
// Via linker map file and xcpclient tool ELF->A2L generation, not supported by Vector A2L Toolset ELF reader
// For struct instance fields, use __ as path separator (params__delay_us means params.delay_us)

// Define physical unit and limits for the calibration parameter 'delay_us' in the params calibration segment
XCP_LIMITS(params__delay_us, 1, 10000);
XCP_UNIT(params__delay_us, "us");

// Define the enum conversion for the calibration parameter test_par_enum
// @@@@ UNUSED: Remove, enum support now build into xcpclient tool
// XCP_UNIT(params__test_par_enum, "0 \"OFF\" 1 \"ON\" 2 \"STANDBY\" ");

//-----------------------------------------------------------------------------------------------------
// Demo global measurement values

// Global measurement variable
// Modified in function foo
// Measuring it in main or task, is possible, but asynchronous and may give inconsistent results
XCP_COMMENT(global_counter, "Global measurement variable"); // Example for meta data annotation as code
uint16_t global_counter = 0;

// A2L Creator code parser annotation
/*
@@ SYMBOL = global_counter
@@ DESCRIPTION = "Global measurement variable"
@@ END
*/

// More global measurement variables of various basic and complex types
uint8_t global_test_uint8 = 8;
uint16_t global_test_uint16 = 16;
uint32_t global_test_uint32 = 32;
uint64_t global_test_uint64 = 64;
int8_t global_test_int8 = -8;
int16_t global_test_int16 = -16;
int32_t global_test_int32 = -32;
int64_t global_test_int64 = -64;
float global_test_float = 0.4f;
double global_test_double = 0.8;
bool global_test_bool = true;
uint8_t global_test_array[3] = {1, 2, 3};
struct test_struct {
    uint16_t a;
    int16_t b;
    float f;
    uint8_t d[3];
};
struct test_struct global_test_struct = {1, -2, 0.3f, {1, 2, 3}};

//-----------------------------------------------------------------------------------------------------
// Demo thread

THREAD_FUNC_RETURN task(void *p) {
    printf("Start thread %u ...\n", get_thread_id());

    // Static local scope measurement variable
    XCP_COMMENT(static_counter, "Static local measurement variable in function task"); // Example for meta data annotation as code
    volatile static uint16_t static_counter = 0;

    // Local measurement variable
    XCP_COMMENT(counter, "Local measurement variable in function task"); // Example for meta data annotation as code
    volatile uint32_t counter = 0;

    DaqCreateEvent(task);

    while (gRun) {

        counter++;
        static_counter++;

        DaqTriggerEvent(task);

        // Sleep for a tunable amount of time (not inside the lock for the calibration parameter block, to not block the XCP server or other threads unnecessarily long)
        uint32_t delay = gCalSeg->lock()->delay_us;
        sleepUs(delay);
    }

    THREAD_FUNC_END; // Exit the thread
}

//-----------------------------------------------------------------------------------------------------
// Demo functions

void foo(void) {

    // Static local scope measurement variable
    volatile static uint16_t static_counter = 0;

    // Local variable
    volatile uint32_t counter = 0;

    // More local measurement variables
    volatile float test_float = 0.1f;
    volatile double test_double = 0.2;
    volatile uint8_t test_uint8 = 1;
    volatile uint16_t test_uint16 = 2;
    volatile uint32_t test_uint32 = 3;
    volatile uint64_t test_uint64 = 4;
    volatile int8_t test_int8 = -1;
    volatile int16_t test_int16 = -2;
    volatile int32_t test_int32 = -3;
    volatile uint64_t test_int64 = 1;
    volatile struct test_struct test_struct = {1, -2, 0.3f, {1, 2, 3}};
    // uint8_t test_array[3] = {1, 2, 3};

    counter = global_counter;
    static_counter = global_counter;

    DaqCreateAndTriggerEvent(foo);
}

// Never called
// Just to demonstrate the DaqCreateAndTriggerEvent macro creates the event, without runing the code in foo
void bar(void) { DaqCreateAndTriggerEvent(bar); }

//-----------------------------------------------------------------------------------------------------
// Demo main

int main(int argc, char *argv[]) {

    std::signal(SIGINT, signal_handler);
    std::signal(SIGTERM, signal_handler);
    std::cout << "\nXCP on Ethernet no_a2l_demo_cpp C++ demo - " << argv[0] << "\n" << std::endl;

    // Set log level (1-error, 2-warning, 3-info, 4-show XCP commands)
    XcpSetLogLevel(OPTION_LOG_LEVEL);

    // Create the EPK software version string in an initialized memory section for offline A2L generation
    // @@@@ XcpCreateEpk(OPTION_PROJECT_VERSION);

    // Initialize the XCP singleton, activate XCP, must be called before starting the server
    // @@@@ TODO: Using binary persistence files not supported, | XCP_MODE_PERSISTENCE
    XcpInit(OPTION_PROJECT_NAME, OPTION_PROJECT_VERSION, XCP_MODE_LOCAL);
    XcpSetElfName(argv[0]); // Set ELF file name for upload via GET_ID, optional with OPTION_ENABLE_ELF_UPLOAD

    // Initialize the XCP Server
    if (!XcpEthServerInit(OPTION_SERVER_ADDR, OPTION_SERVER_PORT, OPTION_USE_TCP, OPTION_QUEUE_SIZE)) {
        return 1;
    }

    // Initialize access to global calibration parameters
    gCalSeg.emplace("params", &kParams);

    // Create threads
    THREAD_HANDLE __t1 = 0;
    create_thread(&__t1, NULL, task, NULL);

    // Local measurement variables
    XCP_COMMENT(counter, "Local measurement variable in main");
    volatile uint16_t counter = 0;
    XCP_COMMENT(static_counter, "Static local measurement variable in main");
    volatile static uint16_t static_counter = 0;

    // Local calibration parameters
    struct counter_control {
        uint16_t counter_max;
        uint16_t counter_inc;
    };
    const struct counter_control kCounterControl = {.counter_max = 1000, .counter_inc = 1};
    // @@@@ CalSegCreate(counter_control);
    xcplib::CalSeg<struct counter_control> calSeg(
        "counter_control",
        &kCounterControl); // Create a local calibration parameter segment for struct 'counter_control' to provide safe and consistent access to the calibration parameters

    // Create a measurement event named "mainloop"
    DaqCreateEvent(mainloop);

    // Mainloop
    printf("Start main loop...\n");
    while (gRun) {

        // Lock the local calibration parameter block calSeg for safe access
        // Calibration segment or block locking is wait-free, locks may be recursive
        // Returns a pointer to the active page (working or reference) of the calibration segment or block
        {
            auto counter_control = calSeg.lock();

            global_counter += counter_control->counter_inc;
            if (global_counter > counter_control->counter_max) { // Limit the global counter with the counter_max calibration value
                global_counter = 0;
            }
        }

        counter = global_counter;
        static_counter = global_counter;

        // Demonstrate calibration thread safety and consistency
        {
            // Lock the global calibration parameter block gCalSeg for safe access
            auto params = gCalSeg->lock();

            if (!((params->test_par_uint64 >> 32) == (params->test_par_uint64 & 0xFFFFFFFF))) {
                printf("Calibration parameter test_par_uint64 is not consistent, value: %016" PRIx64 "\n", params->test_par_uint64);
            }
            if (!(params->test_par_uint32 == params->test_par_uint16)) {
                printf("Calibration parameter test_par_uint32 is not consistently changed with test_par_uint16, value: %08" PRIx32 " != %04" PRIx16 "\n", params->test_par_uint32,
                       params->test_par_uint16);
            }
        }

        // Function calls
        foo(); // Call a function to demonstrate the DaqCreateAndTriggerEvent macro in foo
        // bar(); // Uncomment to demonstrate that the event in bar is created, but the code is never executed, so the event exists, but is never triggered

        // Trigger the measurement event "mainloop"
        DaqTriggerEvent(mainloop);

        // Sleep for a tunable amount of time (not inside the lock for the calibration parameter block, to not block the XCP server or other threads unnecessarily long)
        auto delay = gCalSeg->lock()->delay_us;
        sleepUs(delay);

    } // for (;;)

    // Wait for the thread to stop
    if (__t1)
        join_thread(__t1);

    XcpDisconnect();        // Force disconnect the XCP client
    XcpEthServerShutdown(); // Stop the XCP server

    return 0;
}
