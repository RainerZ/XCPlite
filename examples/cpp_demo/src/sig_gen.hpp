#pragma once

//-----------------------------------------------------------------------------------------------------
// Waveform signal generator class

/*
An instance of class SignalGenerator creates various waveforms, such as sine, square, arbitrary
Depending on calibration parameters ampl, phase, offset and period in struct SignalParametersT
*/

#include <atomic>  // for std::atomic
#include <cstdint> // for uintxx_t
#include <thread>  // for thread

#include "xcplib.hpp" // for xcp::CalSeg

#include "lookup.hpp" // for lookup_table::LookupTableT

// Undefine to demonstrate the alternative way which avoids the invalid offset offset warning 'offset of on non-standard-layout type 'StructType'""
// #define OPTION_USE_SIG_GEN_TYPEDEF // Generate a TYPEDEF for the signal generator class

//-----------------------------------------------------------------------------------------------

namespace signal_generator {

enum SignalTypeT : uint8_t {
    SINE = 0,     // Sine wave
    SQUARE = 1,   // Square wave
    TRIANGLE = 2, // Triangle wave
    SAWTOOTH = 3, // Sawtooth wave
    ARBITRARY = 4 // Arbitrary waveform from lookup table
};

// Signal parameters struct
struct SignalParametersT {
    double ampl;                       // Amplitude
    double phase;                      // Phase shift in radians
    double offset;                     // Offset
    double period;                     // Period in seconds
    lookup_table::LookupTableT lookup; // Lookup table for arbitrary waveforms
    uint32_t delay_us;                 // Delay in microseconds for the task loop
    enum SignalTypeT signal_type;      // Type of the signal (SignalTypeT)
};

class SignalGenerator {

  private:
    xcp::CalSeg<SignalParametersT> signal_parameters_; // Wrapped signal parameters struct to enable XCP calibration access to the instances parameters

    const char *instance_name_;       // Instance name
    std::thread *thread_;             // Thread for the signal generator task
    std::atomic<bool> running_{true}; // Stop flag for the worker thread
    double normalized_time_{0};       // Current normalized time in [0.0..1.0[

    void Task();

  public:
    SignalGenerator(const char *instance_name, const SignalParametersT *params);
    ~SignalGenerator();

    double value_{0};                                        // Current value
    [[nodiscard]] double GetValue() const { return value_; } // Getter for the current value
};

} // namespace signal_generator