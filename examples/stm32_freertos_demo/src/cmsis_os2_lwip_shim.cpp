// Minimal CMSIS-RTOS2 shim for STM32duino LwIP's sys_arch.c.
//
// This is intentionally not a complete CMSIS-RTOS2 implementation. It only
// provides the functions used by the STM32duino LwIP OS port and maps them to
// the FreeRTOS API already used by this demo.

#include <cmsis_os2.h>
#include <FreeRTOS.h>
#include <queue.h>
#include <semphr.h>
#include <task.h>

#include <stdint.h>

extern "C" uint32_t sys_now(void) { return static_cast<uint32_t>(xTaskGetTickCount() * portTICK_PERIOD_MS); }

static TickType_t timeoutToTicks(uint32_t timeout) {
    if (timeout == osWaitForever) {
        return portMAX_DELAY;
    }
    return pdMS_TO_TICKS(timeout);
}

static UBaseType_t priorityToFreeRtos(osPriority_t priority) {
    int32_t value = static_cast<int32_t>(priority);

    // lwIP passes small numeric priorities such as TCPIP_THREAD_PRIO directly.
    // CMSIS priority enums are larger buckets. Both are clamped into the
    // FreeRTOS priority range used by this project.
    if (value <= 0) {
        value = tskIDLE_PRIORITY + 1;
    }
    if (value >= configMAX_PRIORITIES) {
        value = configMAX_PRIORITIES - 1;
    }
    return static_cast<UBaseType_t>(value);
}

extern "C" {

uint32_t osKernelGetTickCount(void) { return static_cast<uint32_t>(xTaskGetTickCount()); }

uint32_t osKernelGetTickFreq(void) { return static_cast<uint32_t>(configTICK_RATE_HZ); }

osThreadId_t osThreadNew(osThreadFunc_t func, void *argument, const osThreadAttr_t *attr) {
    if (func == nullptr) {
        return nullptr;
    }

    const char *name = (attr != nullptr && attr->name != nullptr) ? attr->name : "cmsis";
    const uint32_t stackBytes = (attr != nullptr && attr->stack_size != 0) ? attr->stack_size : 1024;
    const uint32_t stackWords = (stackBytes + sizeof(StackType_t) - 1) / sizeof(StackType_t);
    const osPriority_t priority = (attr != nullptr) ? attr->priority : osPriorityNormal;

    TaskHandle_t handle = nullptr;
    if (xTaskCreate(reinterpret_cast<TaskFunction_t>(func), name, stackWords, argument, priorityToFreeRtos(priority), &handle) != pdPASS) {
        return nullptr;
    }
    return static_cast<osThreadId_t>(handle);
}

osMessageQueueId_t osMessageQueueNew(uint32_t msg_count, uint32_t msg_size, const osMessageQueueAttr_t *attr) {
    (void)attr;
    return static_cast<osMessageQueueId_t>(xQueueCreate(msg_count, msg_size));
}

osStatus_t osMessageQueuePut(osMessageQueueId_t mq_id, const void *msg_ptr, uint8_t msg_prio, uint32_t timeout) {
    (void)msg_prio;
    if (mq_id == nullptr || msg_ptr == nullptr) {
        return osErrorParameter;
    }
    return xQueueSend(static_cast<QueueHandle_t>(mq_id), msg_ptr, timeoutToTicks(timeout)) == pdTRUE ? osOK : osErrorTimeout;
}

osStatus_t osMessageQueueGet(osMessageQueueId_t mq_id, void *msg_ptr, uint8_t *msg_prio, uint32_t timeout) {
    (void)msg_prio;
    if (mq_id == nullptr || msg_ptr == nullptr) {
        return osErrorParameter;
    }
    return xQueueReceive(static_cast<QueueHandle_t>(mq_id), msg_ptr, timeoutToTicks(timeout)) == pdTRUE ? osOK : osErrorTimeout;
}

uint32_t osMessageQueueGetCount(osMessageQueueId_t mq_id) {
    if (mq_id == nullptr) {
        return 0;
    }
    return static_cast<uint32_t>(uxQueueMessagesWaiting(static_cast<QueueHandle_t>(mq_id)));
}

osStatus_t osMessageQueueDelete(osMessageQueueId_t mq_id) {
    if (mq_id == nullptr) {
        return osErrorParameter;
    }
    vQueueDelete(static_cast<QueueHandle_t>(mq_id));
    return osOK;
}

osSemaphoreId_t osSemaphoreNew(uint32_t max_count, uint32_t initial_count, const osSemaphoreAttr_t *attr) {
    (void)attr;
    SemaphoreHandle_t semaphore = xSemaphoreCreateCounting(max_count, initial_count);
    return static_cast<osSemaphoreId_t>(semaphore);
}

osStatus_t osSemaphoreAcquire(osSemaphoreId_t semaphore_id, uint32_t timeout) {
    if (semaphore_id == nullptr) {
        return osErrorParameter;
    }
    return xSemaphoreTake(static_cast<SemaphoreHandle_t>(semaphore_id), timeoutToTicks(timeout)) == pdTRUE ? osOK : osErrorTimeout;
}

osMutexId_t osMutexNew(const osMutexAttr_t *attr) {
    (void)attr;
    return static_cast<osMutexId_t>(xSemaphoreCreateMutex());
}

osStatus_t osMutexAcquire(osMutexId_t mutex_id, uint32_t timeout) {
    if (mutex_id == nullptr) {
        return osErrorParameter;
    }
    return xSemaphoreTake(static_cast<SemaphoreHandle_t>(mutex_id), timeoutToTicks(timeout)) == pdTRUE ? osOK : osErrorTimeout;
}

} // extern "C"
