#ifndef CLOCK64_H
#define CLOCK64_H

#include <stdint.h>

/* Initialize the 64-bit clock. Call once after SystemClock_Config(). */
void Clock64_Init(void);

/* Call from ONE dedicated context to maintain the overflow counter. */
void Clock64_Update(void);

/* Lock-free read — safe from any task or ISR. */
uint64_t Clock64_Get(void);

#endif /* CLOCK64_H */
