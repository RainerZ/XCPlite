#pragma once

#include <stdint.h>

bool startXcpServer();
bool startXcpDemoTasks();

void xcpDemoDisplayUpdate(uint32_t slowTaskPeriodMs, uint16_t slowCounter, uint32_t fastTaskPeriodMs, uint16_t fastCounter);
