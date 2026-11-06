//-----------------------------------------------------------------------------------------------------
// Waveform signal generator class

/*
An instance of class SignalGenerator creates various waveforms, such as sine, square, arbitrary
Depending on calibration parameters ampl, phase, offset and period
*/

#include <cmath>    // for fmod, sin
#include <cstdint>  // for uintxx_t
#include <iostream> // for std::cout
#include <string>   // for std::string
#include <thread>

// Include XCPlite/libxcplite C++ headers
#include "a2l.hpp"    // for A2l generation application programming interface
#include "xcplib.hpp" // for application programming interface

#include "lookup.hpp" // lookup table for arbitrary waveform generator
using namespace lookup_table;

#include "sig_gen.hpp"

namespace signal_generator {

constexpr double kPi = 3.14159265358979323846;
constexpr double k2Pi = (kPi * 2);

// Constructor - creates the signal generator with the given instance name and parameters
SignalGenerator::SignalGenerator(const char *instance_name, const SignalParametersT *params) : signal_parameters_(instance_name, params), instance_name_(instance_name) {

    // Global once A2l registration of SignalParametersT typedef
    if (A2lOnce()) {

        // A2L typedef for the lookup table type
        params->lookup.A2lRegisterTypedef();

        // A2L typedef for the signal generator parameter struct
        A2lCreateEnumConversion(signal_type_enum, "5 0 \"SINE\" 1 \"SQUARE\" 2 \"TRIANGLE\" 3 \"SAWTOOTH\" 4 \"ARBITRARY\"");
        A2lCreateTypedef(SignalParametersT, "A2L typedef for SignalParametersT",                             //
                         A2L_PARAMETER_COMPONENT(signal_type, "Signal type", "conv.signal_type_enum", 0, 4), //
                         A2L_PARAMETER_COMPONENT(ampl, "Amplitude", "Volt", 0, 100),                         //
                         A2L_PARAMETER_COMPONENT(phase, "Phase", "", 0, k2Pi),                               //
                         A2L_PARAMETER_COMPONENT(offset, "Offset", "Volt", -100, 100),                       //
                         A2L_PARAMETER_COMPONENT(period, "Period", "s", 0.01, 10.0),                         //
                         A2L_PARAMETER_COMPONENT(delay_us, "Delay time in us", "us", 0, 100000),             //
                         A2L_TYPEDEF_COMPONENT(lookup, "LookupTableT", 1));                                  // Register member lookup with type LookupTableT

        // A2L typedef for the class SignalGenerator (contains only the members value_ and normalized_time_)
        // Registered from a member function, so private members are accessible
#ifdef OPTION_USE_SIG_GEN_TYPEDEF
        A2lCreateTypedef(SignalGenerator, "A2L typedef for SignalGenerator",                            //
                         A2L_MEASUREMENT_COMPONENT(value_, "Signal generator output", ""),              //
                         A2L_MEASUREMENT_COMPONENT(normalized_time_, "Normalized time in [0..1[", "")); //
#endif
    }

    // Create an A2L instance of this signal generators calibration segment
    signal_parameters_.CreateA2lTypedefInstance("SignalParametersT", "Signal parameters for the signal generator");

    // Start thread
    std::cout << "Starting task: " << instance_name_ << std::endl;
    thread_ = new std::thread([this]() { Task(); });
}

SignalGenerator::~SignalGenerator() {
    running_ = false;
    if (thread_ != nullptr) {
        if (thread_->joinable()) {
            thread_->join();
        }
        delete thread_;
    }
}

// Cyclic calculation function - runs in a separate thread
void SignalGenerator::Task() {

    double time = 0;                                                          // Current time since start in s
    double start_time = static_cast<double>(clockGetMonotonicUs()) / 1000000; // Start time in s

    // Create a measurement event with dynamic name 'instance_name_' for this instance of SignalGenerator
    DaqCreateEvent_s(instance_name_);

    // Register this instance of SignalGenerator
#ifdef OPTION_USE_SIG_GEN_TYPEDEF
    // A2L registration of this complete instance with typedef 'SignalGenerator'
    // @@@@ TODO: Add notes about non standard layout and invalid offset warning
    // Register this instance of SignalGenerator as an instance of the SignalGenerator typedef named '<instance_name_>_generator', relative to this
    // (the name instance_name_ itself is already used by the calibration segment instance)
    std::string generator_instance_name = std::string(instance_name_) + "_generator";
    A2lLock(); // Take care for thread safety when registering measurements and parameters in multiple threads
    A2lSetRelativeAddrMode_s(instance_name_, this);
    A2lCreateInstance(generator_instance_name.c_str(), SignalGenerator, 0, this, "Signal generator instance");
#else
    // A2L registration of individual members of this instance
    // Register the member variable 'value_' of each instance as measurements by using the instance name as a prefix
    A2lLock(); // Take care for thread safety when registering measurements and parameters in multiple threads
    A2lSetRelativeAddrMode_s(instance_name_, this);
    A2lCreatePhysMeasurementInstance(instance_name_, value_, "Signal generator output", "", -100, 100);
#endif

    // Register the local (stack) measurement variable 'time' for each instance
    A2lSetStackAddrMode_s(instance_name_);
    A2lCreatePhysMeasurementInstance(instance_name_, time, "Signal generator time", "s", 0, 3600);
    A2lUnlock();

    while (running_) {

        time = static_cast<double>(clockGetMonotonicUs()) / 1000000 - start_time; // time in s since start of the signal generator

        // Calculate the waveform value based on the current time and signal parameters
        {
            auto p = signal_parameters_.lock();
            normalized_time_ = fmod(time, p->period) / p->period; // Normalize time ([0.0..1.0[ to the period
            double v = 0;
            switch (p->signal_type) {
            case SignalTypeT::SQUARE:
                v = (normalized_time_ < 0.5) ? +1.0 : -1.0;
                break;
            case SignalTypeT::TRIANGLE:
                v = (normalized_time_ - 0.5);
                v = (normalized_time_ < 0.5) ? v : -v;
                v = (v + 0.25) * 4.0;
                break;
            case SignalTypeT::SAWTOOTH:
                v = (normalized_time_ - 0.5) * 2.0;
                break;
            case SignalTypeT::ARBITRARY: {
                // Find the index in the lookup table based on the time
                v = p->lookup.Lookup((float)normalized_time_);
            } break;
            default:
                v = sin((normalized_time_ * k2Pi) + p->phase);
                break;
            }
            value_ = (p->ampl * v) + p->offset;
        }

        // XCP event by event name, event lookup is once and will be cached in a static thread local variable
        DaqTriggerEventExt_s(instance_name_, this); // Trigger with this as dynamic addressing base to make member variables accessible

        // Sleep for delay_us microseconds
        // Be sure the lock is held as short as possible !
        // The calibration segment lock does not content against other threads, it is wait free !
        // But it delays or even starve XCP client tool calibration operations if hold too long
        sleepUs(signal_parameters_.lock()->delay_us);
    }
}

} // namespace signal_generator
