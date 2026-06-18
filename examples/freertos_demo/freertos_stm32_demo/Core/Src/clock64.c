#include "clock64.h"
#include "stm32h7xx_hal.h"

/* Written only by Clock64_Update() (single writer). Read by Clock64_Get(). */
static volatile uint32_t Clock64_HighWord = 0U;

/* Private to Clock64_Update() — not shared with any reader. */
static uint32_t Clock64_PrevLow = 0U;

void Clock64_Init(void)
{
    CoreDebug->DEMCR |= CoreDebug_DEMCR_TRCENA_Msk;
    DWT->CYCCNT = 0U;
    DWT->CTRL  |= DWT_CTRL_CYCCNTENA_Msk;
    Clock64_Update();
}

/*
 * Call from ONE dedicated context (one task or one timer callback).
 * Must be called at least once per DWT rollover period (~8.9 s at 480 MHz).
 */
void Clock64_Update(void)
{
    uint32_t lo = DWT->CYCCNT;
    if (lo < Clock64_PrevLow)
    {
        Clock64_HighWord++;
    }
    Clock64_PrevLow = lo;
}

/*
 * Lock-free read, safe from any task or ISR.
 *
 * Double-read pattern: re-sample HighWord after reading CYCCNT. If it changed,
 * a rollover fired between the two reads — retry. In practice this loop runs
 * exactly once: a retry can only occur once per ~8.9 s.
 */
uint64_t Clock64_Get(void)
{
    uint32_t hi, lo;
    do
    {
        hi = Clock64_HighWord;
        lo = DWT->CYCCNT;
    } while (hi != Clock64_HighWord);
    return ((uint64_t)hi << 32) | lo;
}


