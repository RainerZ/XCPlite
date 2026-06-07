#pragma once

#include <stdint.h>

bool startXcpServer();
bool startXcpDemoTasks();

#ifdef OPTION_DISPLAY
void displayUpdate(uint32_t slowTaskPeriodMs, uint16_t slowCounter, uint32_t fastTaskPeriodMs, uint16_t fastCounter);
#endif
