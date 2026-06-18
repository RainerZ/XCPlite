// On ESP32, use the ESP-IDF high resolution timer (1 us, 64 bit).


#include "esp_timer.h"

#ifdef __cplusplus
extern "C" {
#endif
        
void Clock64_Init(void) {}

void Clock64_Update(void) {}

uint64_t Clock64_Get(void) {
  return (uint64_t)esp_timer_get_time();
}
        
#ifdef __cplusplus
}
#endif

