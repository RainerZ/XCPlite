#pragma once
/*----------------------------------------------------------------------------
| File:
|   util.h
|
| Description:
|   A set of utility functions
|   1. Average and median filter
|   2. Linear regression filter
|   3. Pseudo random number generator
|
|   Code released into public domain, no attribution required
 ----------------------------------------------------------------------------*/

#include <stdbool.h> /* bool   */
#include <stddef.h>  /* size_t */
#include <stdint.h>  /* int64_t, uint64_t */

#ifdef __cplusplus
extern "C" {
#endif

//-------------------------------------------------------------------------------
// Integer Median Filter
//
// Maintains two fixed-size arrays:
//   samples[]  ring buffer storing values in insertion order
//   order[]    indices into samples[], kept in sorted (ascending) order
//
// On each call to median_filter_calc():
//   - The oldest sample is removed from order[] with an O(N) index shift.
//   - The new sample is inserted into order[] at the correct sorted position
//     with an O(N) backwards scan -- no full re-sort.
//   - The median is returned directly from samples[order[count/2]].
//
// Total cost per call: O(N).
// (Adapted from the linuxptp mmedian filter, using static arrays.)
//
// Window size should be odd for an unambiguous middle element.
// With an even window size the average of the two middle elements is returned.

#define MEDIAN_FILTER_MAX_SIZE 31

typedef struct {
    int64_t samples[MEDIAN_FILTER_MAX_SIZE]; /* ring buffer (insertion order) */
    int order[MEDIAN_FILTER_MAX_SIZE];       /* indices sorted by value       */
    int size;                                /* window size (<= MAX_SIZE)  */
    int idx;                                 /* next write position        */
    int count;                               /* samples currently valid    */
} tMedianFilter;

void median_filter_init(tMedianFilter *f, size_t size);
int64_t median_filter_calc(tMedianFilter *f, int64_t v);
size_t median_filter_size(tMedianFilter *f);
size_t median_filter_count(tMedianFilter *f);

//-------------------------------------------------------------------------------------
// Integer Moving Average Filter

#define AVERAGE_FILTER_MAX_SIZE 60
typedef uint64_t tAverageFilterValue;

typedef struct average_filter {
    tAverageFilterValue a[AVERAGE_FILTER_MAX_SIZE]; // circular buffer for values
    tAverageFilterValue as;                         // running sum
    size_t size;                                    // filter window size (max samples)
    size_t ai;                                      // current index in circular buffer
    size_t count;                                   // current number of samples in buffer
} tAverageFilter;

void average_filter_init(tAverageFilter *f, size_t size);
tAverageFilterValue average_filter_calc(tAverageFilter *f, tAverageFilterValue v);
size_t average_filter_size(tAverageFilter *f);
size_t average_filter_count(tAverageFilter *f);
void average_filter_add(tAverageFilter *f, tAverageFilterValue offset);

//-------------------------------------------------------------------------------------
// Double Linreg filter

#define LINREG_FILTER_MAX_SIZE 120

typedef struct linreg_filter {
    double x[LINREG_FILTER_MAX_SIZE]; // circular buffer for x values
    double y[LINREG_FILTER_MAX_SIZE]; // circular buffer for y values
    size_t size;                      // filter window size (max samples)
    size_t ai;                        // current index in circular buffer
    size_t count;                     // current number of samples in buffer

    // State variables for interpolation
    double y_out; // last calculated y output value
    double slope; // last calculated slope
} tLinregFilter;

void linreg_filter_init(tLinregFilter *f, size_t size);
// slope_out is the calculated slope
// y_out is the interpolated y value at x (not the intercept!)
bool linreg_filter_calc(tLinregFilter *f, double x, double y, double *slope_out, double *y_out);
bool linreg_filter_compare(tLinregFilter *f1, tLinregFilter *f2, double x, double *slope_diff, double *y_diff);
size_t linreg_filter_size(tLinregFilter *f);
size_t linreg_filter_count(tLinregFilter *f);

//-------------------------------------------------------------------------------------
// Fast random number generation

// Seed the thread-local splitmix64 PRNG state (optional; auto-initialised)
void fast_rand_seed(uint64_t seed);

// Return a random number in [0, max)
uint64_t fast_rand(uint64_t max);

#ifdef __cplusplus
}
#endif
