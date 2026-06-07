// XCP FreeRTOS demo logic shared by the FreeRTOS examples.

#include <Arduino.h>
#include <math.h>

#include "xcplib.hpp"

#include "xcp_demo.hpp"



//----------------------------------------------------------------------------------------------------
// XCP

#ifndef XCP_PROJECT_NAME
#define XCP_PROJECT_NAME "esp32_freertos_demo"
#endif
#ifndef XCP_PROJECT_VERSION
#define XCP_PROJECT_VERSION "V100"
#endif
#ifndef XCP_USE_TCP
#define XCP_USE_TCP false
#endif
#ifndef XCP_SERVER_PORT
#define XCP_SERVER_PORT 5555
#endif
#ifndef XCP_QUEUE_SIZE
#define XCP_QUEUE_SIZE (1024 * 8)
#endif
#ifndef XCP_LOG_LEVEL
#define XCP_LOG_LEVEL 4 // 3 - Info, 4 - Print XCP commands, 5 - Debug
#endif

bool startXcpServer() {
    const uint8_t bindAny[4] = {0, 0, 0, 0};

    XcpSetLogLevel(XCP_LOG_LEVEL);
    XcpCreateEpk(XCP_PROJECT_VERSION);

    if (!XcpInit(XCP_PROJECT_NAME, XCP_PROJECT_VERSION, XCP_MODE_LOCAL)) {
        Serial.println("XcpInit failed");
        return false;
    }

    if (!XcpEthServerInit(bindAny, XCP_SERVER_PORT, XCP_USE_TCP, XCP_QUEUE_SIZE)) {
        Serial.println("XcpEthServerInit failed");
        return false;
    }

    return true;
}




//----------------------------------------------------------------------------------------------------
// Demo tasks

// LilyGO T-Display-S3 scope hookup:
// channel 1 probe tip -> IO2/GPIO2 header pin, probe ground -> any board GND pin.
// channel 2 probe tip -> IO1/GPIO1 header pin, probe ground -> any board GND pin.
#ifndef FASTTASK_SCOPE_PIN
#define FASTTASK_SCOPE_PIN 2
#endif
#ifndef SLOWTASK_SCOPE_PIN
#define SLOWTASK_SCOPE_PIN 1
#endif

#ifndef DEMO_TASK_CORE
#define DEMO_TASK_CORE 1
#endif

#define FASTTASK_PRIORITY (configMAX_PRIORITIES - 1)
#define SLOWTASK_PRIORITY 3

#if configTICK_RATE_HZ < 1000
#error "fastTask needs configTICK_RATE_HZ >= 1000 for a 1 ms FreeRTOS tick period"
#endif

static constexpr uint32_t FASTTASK_PERIOD_MIN_MS = 1;
static constexpr uint32_t FASTTASK_PERIOD_MAX_MS = 100;
static constexpr uint32_t SLOWTASK_PERIOD_MIN_MS = 1;
static constexpr uint32_t SLOWTASK_PERIOD_MAX_MS = 1000;
static constexpr float SLOWTASK_PHASE_STEP_RAD = 0.1f;
static constexpr float SINE_PERIOD_RAD = 6.28318530717958647692f;

static uint32_t clamp(uint32_t x, uint32_t min, uint32_t max) {
    if (x < min) {
        return min;
    }
    if (x > max) {
        return max;
    }
    return x;
}

// Global measurement values
uint16_t global_counter = 0;
uint32_t fastTaskOverruns = 0;
uint32_t slowTaskOverruns = 0;

// Global calibration parameter constants
struct parameters {
    uint32_t fast_task_period_ms; // Period of measurement task 1 in milliseconds
    uint32_t slow_task_period_ms; // Period of measurement task 2 in milliseconds
    uint16_t counter_max;         // Counter wrap-around value for the global_counter incremented in fastTask
    float amplitude;              // Amplitude for the sine signal generator in slowTask
};

// Default calibration parameters (default/reference page)
// &parameters is the A2l file address of the calibration parameter segment 'parameters'
// Typename and variable name must be identical
const struct parameters parameters = {
    .fast_task_period_ms = 2,  // 2 ms = 500 Hz
    .slow_task_period_ms = 10, // 10 ms = 100 Hz
    .counter_max = 1000,
    .amplitude = 1.0f,
};

// Declare a calibration segment that wraps 'parameters' for thread-safe and consistent access.
// This creates:
//  - a linker-section 'xcp_cals' descriptor used by XcpInit() for registration
//  - an internal calibration segment index initialized by XcpInit()
//  - the typed C++ handle 'parameters_calseg' used by the tasks below
// The offline A2L generator currently assumes that the struct type name and default-parameter variable name are identical.
CalSegDeclRef(parameters, parameters_calseg);

TaskHandle_t fastTaskHandle = nullptr;
TaskHandle_t slowTaskHandle = nullptr;

static BaseType_t createDemoTask(TaskFunction_t taskCode, const char *name, const uint32_t stackDepth, UBaseType_t priority, TaskHandle_t *taskHandle) {
#ifdef ARDUINO_ARCH_ESP32
    return xTaskCreatePinnedToCore(taskCode, name, stackDepth, nullptr, priority, taskHandle, DEMO_TASK_CORE);
#else
    return xTaskCreate(taskCode, name, stackDepth, nullptr, priority, taskHandle);
#endif
}

// High priority fast task
void fastTask(void *parameter) {
    (void)parameter;

    // Volatile keeps this local measurement variable visible in optimized builds,
    // The offline A2L generator can discover it in the ELF file and associate it to the functions DAQ event trigger.
    volatile uint16_t counter = 0;

#ifdef OPTION_SERIAL_PRINTF
    Serial.printf("fastTask started\n");
    Serial.printf("  priority = %u\n", static_cast<unsigned>(uxTaskPriorityGet(nullptr)));
    Serial.printf("  frameaddr = %p\n", xcp_get_frame_addr());
    Serial.printf("  &counter = %p\n", &counter);
#endif

    // Create a DAQ event named 'fastTask'
    DaqCreateEvent(fastTask);

    // Initialize IO pin
    pinMode(FASTTASK_SCOPE_PIN, OUTPUT);
    digitalWrite(FASTTASK_SCOPE_PIN, LOW);

    TickType_t lastWakeTime = xTaskGetTickCount();
    for (;;) {

        // Toggle an IO pin to observe cycle time jitter and runtime jitter
        digitalWrite(FASTTASK_SCOPE_PIN, HIGH);

        uint32_t period_ms;

        // Lock the calibration segment 'parameters' for thread-safe and consistent access
        // There is no blocking mutex hold during the lock, only atomics used.
        {
            auto params = parameters_calseg.lock();

            // Save the task period parameter, don't delay during the lock to give XCP a chance to modify the parameters.
            period_ms = clamp(params->fast_task_period_ms, FASTTASK_PERIOD_MIN_MS, FASTTASK_PERIOD_MAX_MS);

            counter++;
            if (counter > params->counter_max) {
                counter = 0;
            }
            global_counter++;
            if (global_counter > params->counter_max) {
                global_counter = 0;
            }
        }

        // Trigger the DAQ event 'fastTask'
        DaqTriggerEvent(fastTask);

        // Toggle IO pin
        digitalWrite(FASTTASK_SCOPE_PIN, LOW);

        // Sleep until next wakeup time, check for overruns
        const BaseType_t delayed = xTaskDelayUntil(&lastWakeTime, pdMS_TO_TICKS(period_ms));
        if (delayed == pdFALSE) {
            fastTaskOverruns++;
        }
    }
}

// Low priority slow task
void slowTask(void *parameter) {
    (void)parameter;

    volatile uint16_t counter = 0;
    volatile float sineValue = 0.0f;
    float phase = 0.0f;
    TickType_t lastWakeTime = xTaskGetTickCount();

#ifdef OPTION_SERIAL_PRINTF
    Serial.printf("slowTask started\n");
    Serial.printf("  frameaddr = %p\n", xcp_get_frame_addr());
    Serial.printf("  &counter = %p\n", &counter);
#endif

    DaqCreateEvent(slowTask);
    pinMode(SLOWTASK_SCOPE_PIN, OUTPUT);
    digitalWrite(SLOWTASK_SCOPE_PIN, LOW);

    for (;;) {

        digitalWrite(SLOWTASK_SCOPE_PIN, HIGH);

        uint32_t slow_task_period_ms;
        uint32_t fast_task_period_ms;

        {
            auto params = parameters_calseg.lock();
            slow_task_period_ms = clamp(params->slow_task_period_ms, SLOWTASK_PERIOD_MIN_MS, SLOWTASK_PERIOD_MAX_MS);
            fast_task_period_ms = clamp(params->fast_task_period_ms, FASTTASK_PERIOD_MIN_MS, FASTTASK_PERIOD_MAX_MS);

            counter++;
            if (counter > params->counter_max) {
                counter = 0;
            }

            sineValue = params->amplitude * sinf(phase);
            phase += SLOWTASK_PHASE_STEP_RAD;
            if (phase >= SINE_PERIOD_RAD) {
                phase -= SINE_PERIOD_RAD;
            }
        }

        DaqTriggerEvent(slowTask);

#ifdef OPTION_SERIAL_PRINTF
        Serial.printf("slowTask: core %d - %u, period = %u ms, sine = %.3f\n", xPortGetCoreID(), counter, static_cast<unsigned>(slow_task_period_ms),
                      static_cast<double>(sineValue));
#endif

#ifdef OPTION_DISPLAY
        displayUpdate(slow_task_period_ms, counter, fast_task_period_ms, global_counter);
#endif

        digitalWrite(SLOWTASK_SCOPE_PIN, LOW);

        const BaseType_t delayed = xTaskDelayUntil(&lastWakeTime, pdMS_TO_TICKS(slow_task_period_ms));
        if (delayed == pdFALSE) {
            slowTaskOverruns++;
        }
    }
}

bool startXcpDemoTasks() {
    Serial.printf("&global_counter = %p\n", &global_counter);
    Serial.printf("&parameters = %p\n", &parameters);

    BaseType_t taskCreated = createDemoTask(fastTask, "fastTask",
                                            2048, // stack
                                            FASTTASK_PRIORITY, &fastTaskHandle);
    if (taskCreated != pdPASS) {
        Serial.println("Failed to create fastTask");
        return false;
    }

    taskCreated = createDemoTask(slowTask, "slowTask",
                                 4096, // stack
                                 SLOWTASK_PRIORITY, &slowTaskHandle);
    if (taskCreated != pdPASS) {
        Serial.println("Failed to create slowTask");
        return false;
    }

    return true;
}
