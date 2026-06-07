#include <Arduino.h>
#include <STM32FreeRTOS.h>

#include "stm32_lwip_ethernet.hpp"
#include "xcp_demo.hpp"

static void appTask(void *parameter) {
    (void)parameter;

    Serial.println();
    Serial.println("STM32 FreeRTOS XCP demo");

    if (!stm32StartEthernet()) {
        Serial.println("Ethernet startup failed. XCP server not started.");
    } else if (!startXcpServer()) {
        Serial.println("XCP server startup failed.");
    } else {
        Serial.printf("XCP server listening on %s:5555 UDP\n", stm32EthernetIpString());
    }

    if (!startXcpDemoTasks()) {
        Serial.println("XCP demo task startup failed.");
    }

    vTaskDelete(nullptr);
}

void setup() {
    Serial.begin(115200);
    delay(500);

    if (xTaskCreate(appTask, "app", 4096, nullptr, 3, nullptr) != pdPASS) {
        Serial.println("Failed to create app task");
        return;
    }

    vTaskStartScheduler();
    Serial.println("FreeRTOS scheduler failed to start.");
}

void loop() {
    vTaskDelay(pdMS_TO_TICKS(1000));
}
