// no_a2l_demo_cpp - XCPlite example
// Demonstrates XCPlite operation without runtime A2L generation
// This concept works on microcontrollers and microprocessors without filesystem support
// Requires manual or tool based XCPlite specific A2L file creation and update process or direct ELF support
// See ../README.md for details

#include <array>    // for std::array
#include <atomic>   // for std::atomic
#include <csignal>  // for signal handling
#include <cstdint>  // for uintxx_t
#include <iostream> // for std::cout
#include <optional> // for std::optional

#include "xcplib.hpp" // for libxcplite application programming interface

// Internal libxcplite includes to simplify multi platform support and keep the demo code simple and readable
#include "platform.h" // for platform abstraction - thread local, threads, mutex, sockets, sleepUs, ...

// Signal handler for graceful exit on Ctrl+C
std::atomic<bool> gRun{true};
void signal_handler(int signal) {
    if (signal == SIGINT || signal == SIGTERM) {
        gRun = false;
    }
}

//-----------------------------------------------------------------------------------------------------
// XCP configuration parameters

#define OPTION_PROJECT_VERSION "109" // EPK version string

constexpr const char OPTION_PROJECT_NAME[] = "no_a2l_demo_cpp"; // Project name, used to build the A2L and BIN file name
constexpr bool OPTION_USE_TCP = false;                          // TCP or UDP
constexpr uint8_t OPTION_SERVER_ADDR[] = {0, 0, 0, 0};          // Bind addr, 0.0.0.0 = ANY
constexpr uint16_t OPTION_SERVER_PORT = 5555;                   // Port
constexpr uint16_t OPTION_QUEUE_SIZE = (1024 * 32);             // Size of the queue in bytes, should be large enough to cover at least 10ms of expected traffic
constexpr int OPTION_LOG_LEVEL = 3;                             // Log level, 0 = no log, 1 = error, 2 = warning, 3 = info, 4 = debug

//-----------------------------------------------------------------------------------------------------
// Demo global calibration parameters

// Global calibration parameter set type defined as a struct with various basic and complex types
struct params {

    uint32_t delay_us; // Sleep time in microseconds for the main and the task loop

    // Calibration parameters of various basic and complex types (for demonstration )
    double test_par_double;
    bool test_par_bool;
    uint64_t test_par_uint64;
    uint32_t test_par_uint32;
    uint16_t test_par_uint16;
    uint8_t test_par_uint8_array[10];
    struct test_par_struct {
        uint16_t test_field_uint16;
        int16_t test_field_int16;
        float test_field_float;
        uint8_t test_field_uint8_array[3];
    } test_par_struct;
};

// Default values for the calibration parameters, used as default/reference page for the calibration parameter segment in A2L/XCP
// Note: calibration segment defaults must have static lifetime and addressable storage for A2L/XCP to work !!
const struct params params = {.delay_us = 1000,
                              .test_par_double = 0.123456789,
                              .test_par_bool = true,
                              .test_par_uint64 = 0x1234567812345678,
                              .test_par_uint32 = 0x1234,
                              .test_par_uint16 = 0x1234,
                              .test_par_uint8_array = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9},
                              .test_par_struct = {2, -2, 0.4f, {0, 1, 2}}};

// Create a global visibility and static lifetime calibration parameter segment named `params'
// The calibration segment has a working and a default/reference page
// With the calibration parameter constants in `params` as default/reference page
// The address of `params` will be the A2L address of the calibration segment (MEMORY_SEGMENT) and its instance
// The calibration segment wraps the constant 'params' for thread-safe and consistent access (lockless, wait-free RCU algorithm).
// This creates:
//  - a rodata linker-section 'xcp_cals' descriptor, at build-time used by the ELF->A2L generator, and at runtime used by XcpInit() for early registration
//  - a runtime calibration segment RCU initialized at runtime by XcpInit()
//  - a code definition for a typed C++ handle named 'params_calseg' (naming convention '<struct_name>_calseg') with static lifetime and file scope, used by the demo code below
// Note: The offline A2L generator currently assumes that the struct type name (`struct params`) and the calibration segment name (`params`) are identical.
CalSegDecl(params);

// Metadata annotation option 1:
// Metadata annotation as code (static data in a special ELF section)
// Via linker map file and xcpclient tool ELF->A2L generation, not supported by Vector A2L Toolset ELF reader
// For struct instance fields, use __ as path separator (params__delay_us means params.delay_us)

// Define physical unit and limits for the calibration parameter 'delay_us' in the params calibration segment
XCP_LIMITS(params__delay_us, 1, 10000);
XCP_UNIT(params__delay_us, "us");

// Metadata annotation option 2:
// Metadata annotation as comments for Vector A2L Toolset Creator
// Example metadata annotations For the calibration parameter 'delay_us' in the params calibration segment
/*
@@ STRUCTURE = params
@@ ELEMENT = delay_us
@@ DESCRIPTION = "Sleep time in microseconds for the main and the task loop"
@@ UNIT = "us"
@@ LIMITS = 1.0, 10000.0
@@ DATA_TYPE = UWORD [0 ... 10000]
@@ END
*/

//-----------------------------------------------------------------------------------------------------
// Demo global measurement values

// Global measurement variable
uint16_t global_counter = 0;

// Meta data annotation as code
// Modified in function foo, measuring it in main or task, is possible, but asynchronous and may give inconsistent results
XCP_COMMENT(global_counter, "Global measurement variable"); // Example for meta data annotation as code

// Meta data annotation for Vector A2L Creator
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
// Demo class

enum CounterState { OFF = 0, ON = 1, RESET = 2 };

// Paremeterset type for the counter controller
template <typename CounterType> struct CounterControllerParamsTemplate {
    CounterType max;
    CounterType inc;
    CounterState state;
};

// Parameterized counter controller class template over the counter type and the parameter set type
template <typename CounterType, typename ParamsType> class CounterController {
  public:
    using CalSegHandle = xcp::CalSegRef<const ParamsType>;

    // The constructor takes a calibration segment handle
    explicit CounterController(const CalSegHandle &calseg) : calseg_(calseg) {}

    // Step the counter value, using the calibration parameters
    void step(CounterType &value) const {
        auto cal = calseg_.lock();
        if (cal->state == ON) {
            value = value + cal->inc;
        }
        if (value > cal->max || cal->state == RESET) {
            value = 0;
        }
    }

  private:
    // The calibration segment handle is stored as a member variable, and is used to access the calibration parameters in a thread-safe and consistent manner
    const CalSegHandle &calseg_;
};

//-----------------------------------------------------------------------------------------------------
// Demo class instance of 'CounterController' named 'counter_controller' with calibration parameters in 'counter_controller_params'
// A global CounterController instance with a global calibration parameters segment 'CounterControllerParams'

// Create a template alias for the counter controller parameters for the ELF->A2L generator
// The A2L generator does not yet automatically detect the template parameters and mangle type names for A2L
using CounterControllerParams = CounterControllerParamsTemplate<uint16_t>;

// Default values for CounterControllerParams, used as default/reference page for the calibration parameter segment in A2L/XCP
// Note: calibration segment defaults must have static lifetime and addressable storage for A2L/XCP to work !!
static const CounterControllerParams counter_controller_params = {.max = 1000, .inc = 1, .state = ON};

// Create a global visibility and static lifetime calibration segment named 'counter_controller_params'
// With the calibration parameter constants in `counter_controller_params` as default/reference page
// The address of `counter_controller_params` will be the A2L address of the calibration segment and its instance
// Note: The offline A2L generator currently assumes that the struct type name and default-parameter variable name (`params`) are identical.
CalSegDeclRef(counter_controller_params, counter_control_calseg_handle);

// Create a global counter controller class instance for uint16_t counters and inject the calibration parameter segment handle for its parameters
CounterController<uint16_t, CounterControllerParams> counter_controller(counter_control_calseg_handle);

//-----------------------------------------------------------------------------------------------------
// Demo thread

THREAD_FUNC_RETURN task(void *p) {
    printf("Start thread %u ...\n", get_thread_id());

    // Create a template alias name for the ELF->A2L generator
    using TaskCounterControllerParams = CounterControllerParamsTemplate<uint32_t>;

    // Create a local visibility, but static lifetime calibration segment named 'task_counter_controller'
    // With the calibration parameter constants in `task_counter_controller` as default/reference page
    // The address of `task_counter_controller` will be the A2L address of the calibration segment and its instance
    // Note: local visibility is fine here because the object still has static storage duration.
    // Note: The offline A2L generator assumes that the paramneter struct instance name 'task_counter_controller' and the segment name (`task_counter_controller`) are identical.
    static const TaskCounterControllerParams task_counter_controller_params = {.max = 100, .inc = 10, .state = ON};
    CalSegDeclRef(task_counter_controller_params, counter_control_task_calseg_handle);

    // Create a task-local counter controller instance with a different counter type
    CounterController<uint32_t, TaskCounterControllerParams> counter_controller(counter_control_task_calseg_handle);

    // Static local scope measurement variable
    XCP_COMMENT(static_counter, "Static local measurement variable in function task"); // Example for meta data annotation as code
    static uint32_t static_counter = 0;

    // Local measurement variable
    XCP_COMMENT(counter, "Local measurement variable in function task"); // Example for meta data annotation as code
    uint32_t counter = 0;

    // Create a section registered measurement event named "task"
    DaqCreateEvent(task);

    while (gRun) {

        // Operate the local counters using the local counter controller instance
        counter_controller.step(counter);
        counter_controller.step(static_counter);

        // Trigger the measurement event "task"
        DaqTriggerEvent(task);

        // Sleep for a tunable amount of time (not inside the lock for the calibration parameter block, to not block the XCP server or other threads unnecessarily long)
        uint32_t delay = params_calseg.lock()->delay_us;
        sleepUs(delay);
    }

    THREAD_FUNC_END; // Exit the thread
}

//-----------------------------------------------------------------------------------------------------
// Demo functions

void foo(void) {

    // Static local scope measurement variable
    static uint16_t static_counter = 0;

    // Local variable
    uint16_t counter = 0;

    // Operate the local counters using the global counter controller instance
    counter_controller.step(counter);
    counter_controller.step(static_counter);

    // More local measurement variables
    volatile float test_float = 0.001f * counter;
    volatile double test_double = 0.001 * counter;
    volatile uint8_t test_uint8 = 1;
    volatile uint16_t test_uint16 = 2;
    volatile uint32_t test_uint32 = 3;
    volatile uint64_t test_uint64 = 4;
    volatile int8_t test_int8 = -1;
    volatile int16_t test_int16 = -2;
    volatile int32_t test_int32 = -3;
    volatile uint64_t test_int64 = 1;
    volatile struct test_struct test_struct = {1, -2, 0.001f * counter, {1, 2, 3}};
    // uint8_t test_array[3] = {1, 2, 3};

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
    XcpCreateEpk(OPTION_PROJECT_VERSION);

    // Initialize the XCP singleton, activate XCP, must be called before starting the server
    XcpInit(OPTION_PROJECT_NAME, OPTION_PROJECT_VERSION, XCP_MODE_LOCAL);
    XcpSetElfName(argv[0]); // Set ELF file name for upload via GET_ID, optional

    // Initialize the XCP Server
    if (!XcpEthServerInit(OPTION_SERVER_ADDR, OPTION_SERVER_PORT, OPTION_USE_TCP, OPTION_QUEUE_SIZE)) {
        return 1;
    }

    // Create threads
    THREAD_HANDLE __t1 = 0;
    create_thread(&__t1, NULL, task, NULL);

    // Local measurement variables
    XCP_COMMENT(counter, "Local measurement variable in main");
    uint16_t counter = 0;
    XCP_COMMENT(static_counter, "Static local measurement variable in main");
    static uint16_t static_counter = 0;

    // Create a section registered measurement event named "mainloop"
    DaqCreateEvent(mainloop);

    // Mainloop
    printf("Start main loop...\n");
    while (gRun) {

        counter_controller.step(global_counter);
        counter_controller.step(counter);
        counter_controller.step(static_counter);

        // Demonstrate calibration thread safety and consistency (typical concern on 32 bit microcontrollers)
        {
            // Lock the global calibration parameter block gCalSeg for safe access
            auto params = params_calseg.lock();

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
        auto delay = params_calseg.lock()->delay_us;
        sleepUs(delay);

    } // for (;;)

    // Wait for the thread to stop
    if (__t1)
        join_thread(__t1);

    XcpDisconnect();        // Force disconnect the XCP client
    XcpEthServerShutdown(); // Stop the XCP server

    return 0;
}
