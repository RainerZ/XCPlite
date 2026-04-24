/*----------------------------------------------------------------------------
| File:
|   sync.c
|
| Description:
|   Lightweight clock synchronizer.
|
|   Interpolates a target clock (t1) from a reference clock (t2) using
|   consecutive (t1, t2) timestamp pairs, e.g. hardware/software timestamp
|   pairs obtained via Linux SO_TIMESTAMPING on an Ethernet interface.
|
|   Epoch independence:
|     t1 and t2 do NOT need to share the same epoch.
|
|   Algorithm (SYNC_MODE_DEFAULT):
|     After two pairs the drift ratio is estimated as:
|
|       drift_ppb = (dt1 - dt2) * 1_000_000_000 / dt2
|
|     where dt1 = t1_new - t1_old  and  dt2 = t2_new - t2_old.
|
|     Interpolation from an arbitrary t2 is then:
|
|       t1_est = t1_anchor + dt2 + dt2 * drift_ppb / 1_000_000_000
|
|     All arithmetic is in int64_t.  dt2 is always a small inter-packet
|     interval (< ~10 s), so there is no overflow risk.
|
|   Algorithm (SYNC_MODE_PI):
|     A PI (proportional-integral) servo maintains drift_ppb as the sum
|     of two terms:
|
|       drift_ppb = pi_drift  +  error_ns / 2^kp_shift
|                  (integral)    (proportional)
|
|     where  error_ns = t1_actual - t1_predicted  (offset error in ns).
|
|     The integral accumulator is updated each cycle:
|
|       pi_error_accum += error_ns              (raw ns, avoids dead zone)
|       pi_drift        = pi_error_accum / 2^ki_shift
|
|     Accumulating raw errors before dividing eliminates the dead zone that
|     would occur if small errors were truncated to zero before accumulation.
|
|     The proportional term reacts immediately to offset errors;
|     the integral term accumulates and tracks slowly changing drift
|     (e.g. due to oscillator temperature coefficient).
|
|     Bootstrap: the first two pairs use the DEFAULT 2-point estimator
|     to seed pi_drift; PI updates begin from the third pair onwards.
|
|     Overflow analysis (SYNC_MODE_PI):
|       error_ns    <= ~100,000 ns  (100 us -- extreme SW timestamp jitter)
|       pi_drift    <= 500,000 ppb  (clamped at 500 ppm)
|       drift_ppb   <= 600,000 ppb  (integral + proportional)
|       dt2*drift   <= 10e9 * 6e5 = 6e15  << INT64_MAX  -> safe
|
|  Code released into public domain, no attribution required
|
 ----------------------------------------------------------------------------*/

#include "sync.h"

#include <assert.h>   /* assert                */
#include <inttypes.h> /* PRId64, PRIu64        */
#include <stdbool.h>  /* bool                  */
#include <stdint.h>   /* uint64_t, int64_t ... */
#include <stdio.h>    /* printf                */
#include <string.h>   /* memset                */

#include "dbg_print.h" // for DBG_PRINTF
#include "util.h"      // for tMedianFilter, median_filter_*

/* ---------------------------------------------------------------------------
 * syncInit
 * -------------------------------------------------------------------------*/
void syncInit(tClockSynchronizer *s, uint8_t mode) {
    assert(s != NULL);
    memset(s, 0, sizeof(*s));
    s->mode = mode;
    if (mode == SYNC_MODE_PI) {
        s->kp_shift = SYNC_PI_KP_SHIFT_DEFAULT;
        s->ki_shift = SYNC_PI_KI_SHIFT_DEFAULT;
    }
    /* Always initialise the median filter so it is ready to use.
     * It is disabled by default (use_median == false after memset).
     * Enable: s->use_median = true;
     * Resize: median_filter_init(&s->median_filter, N);  */
    median_filter_init(&s->median_filter, SYNC_MEDIAN_WINDOW_DEFAULT);
}

/* ---------------------------------------------------------------------------
 * syncState
 * -------------------------------------------------------------------------*/
bool syncState(const tClockSynchronizer *s) {
    assert(s != NULL);
    return s->is_sync;
}

/* ---------------------------------------------------------------------------
 * syncInterpolateT1FromT2
 *
 * Interpolate t1 (target clock) from t2 (reference clock).
 * All arithmetic in int64_t for precision and performance.
 *
 * Overflow analysis for  dt2 * drift_ppb / 1e9:
 *   dt2          <= ~10 s  = 10_000_000_000 ns  (normal call rate >> 0.1 Hz)
 *   |drift_ppb|  <=  1_000_000 ppb  (1000 ppm -- extreme clock error)
 *   product      <= 10e9 * 1e6 = 1e16  << INT64_MAX (9.2e18)  -> safe
 * -------------------------------------------------------------------------*/
uint64_t syncInterpolateT1(tClockSynchronizer *s, uint64_t t2) {
    assert(s != NULL);
    assert(s->is_sync);

    /* Elapsed time on the reference clock since the anchor in t2 */
    int64_t dt2 = (int64_t)(t2 - s->t2);

    /*
     * Scale dt2 by the clock ratio:
     *   dt1 = dt2 * (1 + drift_ppb / 1e9)
     *       = dt2 + dt2 * drift_ppb / 1_000_000_000
     */
    int64_t dt1 = dt2 + dt2 * s->drift_ppb / 1000000000LL;
    uint64_t t1_est = (uint64_t)((int64_t)s->t1 + dt1);

    /* Guarantee monotonically increasing output */
    if (t1_est <= s->last_t1_out) {
        t1_est = s->last_t1_out + 1;
        DBG_PRINTF_WARNING("syncInterpolateT1FromT2: non-monotonic output adjusted to %" PRIu64 "\n", t1_est);
    }
    s->last_t1_out = t1_est;

    return t1_est;
}

/* ---------------------------------------------------------------------------
 * syncUpdate
 *
 * Feed a new (t1, t2) timestamp pair.
 * Calculate drift and update the interpolation anchor.
 *
 * On the first call only the anchor is stored.
 * On the second call is_sync is set to true (both modes).
 * From the third call onwards, SYNC_MODE_PI drives drift_ppb via the PI servo.
 *
 * Invalid pairs (non-monotonic timestamps) are silently dropped.
 * -------------------------------------------------------------------------*/
void syncUpdate(tClockSynchronizer *s, uint64_t t1, uint64_t t2) {
    assert(s != NULL);

    if (s->cycle_count == 0) {
        /* First call: store anchor only, nothing to compute yet. */
        s->t1 = t1;
        s->t2 = t2;
        s->cycle_count++;
        return;
    }

    int64_t dt2 = (int64_t)(t2 - s->t2);
    int64_t dt1 = (int64_t)(t1 - s->t1);

    /* Reject non-monotonic pairs. */
    if (dt2 <= 0 || dt1 <= 0)
        return;

    if (!s->is_sync) {
        /*
         * Bootstrap (both modes): compute drift from the first valid pair
         * using the simple 2-point estimator.
         *
         *   drift_ppb = (dt1 - dt2) * 1_000_000_000 / dt2
         *
         * For SYNC_MODE_PI, pi_drift is seeded with this value so that the
         * servo starts from a good initial state rather than from zero.
         */
        int64_t raw_drift = (dt1 - dt2) * 1000000000LL / dt2;
        s->drift_ppb = raw_drift;
        if (s->mode == SYNC_MODE_PI) {
            s->pi_drift = raw_drift;
            /* Seed the raw accumulator consistently with pi_drift so that the
             * first PI update starts from the right state.                  */
            s->pi_error_accum = raw_drift * (1LL << s->ki_shift);
        }
        s->is_sync = true;

    } else if (s->mode == SYNC_MODE_PI) {
        /*
         * PI servo update.
         *
         * Compute the offset error: how far is the actual t1 from the value
         * our current model would have predicted at this t2?
         *
         *   t1_predicted = t1_anchor + dt2 + dt2 * drift_ppb / 1e9
         *   error_ns     = t1_actual - t1_predicted
         *                = dt1 - (dt2 + dt2 * drift_ppb / 1e9)
         *
         * Positive error: t1 is ahead of prediction (t1 runs faster than
         * our current drift model; drift_ppb should increase).
         */
        int64_t dt1_pred = dt2 + dt2 * s->drift_ppb / 1000000000LL;
        int64_t error_ns = dt1 - dt1_pred;

        /*
         * Optional median pre-filter: reject outlier pairs before they enter
         * the PI servo.  A single wildly wrong hardware timestamp has zero
         * effect as long as fewer than window/2 consecutive pairs are bad.
         */
        if (s->use_median)
            error_ns = median_filter_calc(&s->median_filter, error_ns);

        /*
         * Integral term: accumulate raw error in ns, then derive pi_drift.
         *
         * Accumulating before dividing avoids a dead zone: with divide-first,
         * any |error_ns| < 2^ki_shift would truncate to zero and a small but
         * persistent offset (e.g. from temperature-induced drift) would never
         * move the integral.  Accumulating first means even a 1 ns persistent
         * error shifts pi_drift after enough cycles.
         *
         * Anti-windup (back-calculation): clamp the accumulator to the range
         * that corresponds to +/- SYNC_PI_INTEGRAL_CLAMP ppb in pi_drift.
         * Clamping the accumulator (not just the output) prevents it from
         * winding further in saturation.
         */
        int64_t accum_clamp = SYNC_PI_INTEGRAL_CLAMP * (1LL << s->ki_shift);
        s->pi_error_accum += error_ns;
        if (s->pi_error_accum > accum_clamp)
            s->pi_error_accum = accum_clamp;
        if (s->pi_error_accum < -accum_clamp)
            s->pi_error_accum = -accum_clamp;
        s->pi_drift = s->pi_error_accum / (1LL << s->ki_shift);

        /*
         * Proportional term: immediate correction for current offset error.
         * Does not accumulate; adds noise proportional to jitter.
         */
        s->drift_ppb = s->pi_drift + error_ns / (1LL << s->kp_shift);

    } else {
        /*
         * SYNC_MODE_DEFAULT: re-estimate drift from every consecutive pair.
         * No filtering -- optimal for very low-jitter sources.
         *
         * Optional median pre-filter: feed the raw per-pair drift estimate
         * through the median filter.  The median of the last N estimates is
         * used as drift_ppb, rejecting occasional outlier pairs.
         */
        int64_t raw_drift = (dt1 - dt2) * 1000000000LL / dt2;
        DBG_PRINTF3("syncUpdate: dt1 = %" PRId64 ", dt2 = %" PRId64 ", raw_drift = %" PRId64 " ppb\n", dt1, dt2, raw_drift);
        s->drift_ppb = s->use_median ? median_filter_calc(&s->median_filter, raw_drift) : raw_drift;
    }

    /* Advance the interpolation anchor to the new pair. */
    s->t1 = t1;
    s->t2 = t2;
    s->cycle_count++;
}

/* ---------------------------------------------------------------------------
 * syncSet
 *
 * Feed a new (t1,t2) timestamp anchor and set drift parameters directly.
 * Used for testing and simulation of clock properties
 * -------------------------------------------------------------------------*/
void syncSet(tClockSynchronizer *s, uint64_t t1, int64_t t2, int64_t drift_ppb) {
    assert(s != NULL);
    s->t1 = t1;
    s->t2 = t2;
    s->drift_ppb = drift_ppb;
    s->is_sync = true;
}
