// clock64.h - 64-bit microsecond-resolution clock using DWT cycle counter.

#ifndef CLOCK64_H
#define CLOCK64_H

#include <stdint.h>


// If not defined, Clock64_Get() returns raw DWT ticks as 64 bit value 
// With this defined, it returns microseconds (rounded down) using compile-time constants and integer math — no runtime division or floating-point, and no cumulative drift beyond the resolution of the conversion.
#define CONVERT_TO_US 
#define CLOCK_TICKS_PER_S 480000000UL // DWT clock frequency — must be an integer multiple of 1 MHz if CONVERT_TO_US is defined



/* Initialize the 64-bit clock. Call once after SystemClock_Config(). */
void Clock64_Init(void);

/* Call from ONE dedicated context to maintain the overflow counter. */
void Clock64_Update(void);

/* Lock-free read — safe from any task or ISR. */
uint64_t Clock64_Get(void);

#endif /* CLOCK64_H */
