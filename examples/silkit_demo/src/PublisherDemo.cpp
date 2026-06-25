// silkit_demo - Publisher application

#include "ApplicationBase.hpp"
#include "PubSubDemoCommon.hpp"
#include "XcpHelper.hpp"
#include <cmath>

using namespace PubSubDemoCommon;
using namespace xcplib; // For CalSeg

//-----------------------------------------------------------------------------------------------------
// Tunable and persistent calibration parameters

struct ParametersT {
    uint16_t counter_max;    // Maximum value for the step loop counter
    uint32_t delay_us;       // Sleep time in microseconds for DoWorkSync
    double signal_amplitude; // Amplitude for the simulated GPS signal strength
    bool use_simulated_time; // Whether to use simulated time for XCP events or real time
};

// Default values
const ParametersT kParameters = {.counter_max = 1000, .delay_us = 1000, .signal_amplitude = 1.0, .use_simulated_time = true};

//-----------------------------------------------------------------------------------------------------
// Demo global measurement values

class Publisher : public ApplicationBase {
  public:
    Publisher(Arguments args = Arguments{}) : ApplicationBase(args), _parameters("kParameters", &kParameters) {}
    ~Publisher() { XcpServerShutdown(); }

  private:
    IDataPublisher *_gpsPublisher;
    IDataPublisher *_temperaturePublisher;

    // Tunable and persistent calibration parameters
    CalSeg<ParametersT> _parameters;

    // Member variables for measurements
    uint16_t _counter = 0;
    GpsData _gps_data = {0.0, 0.0, 0.0};
    double _temperature = 0.0;

    void AddCommandLineArgs() override {}

    void EvaluateCommandLineArgs() override {}

    // ----------------------------------------------------------------
    void CreateControllers() override {

        // Create data publishers for GPS and temperature data
        _gpsPublisher = GetParticipant()->CreateDataPublisher("GpsPublisher", dataSpecGps, 0);
        _temperaturePublisher = GetParticipant()->CreateDataPublisher("TemperaturePublisher", dataSpecTemperature, 0);

        // Create a typedef for struct GpsData
        // (Variadic template style API)
        A2lCreateTypedef(GpsData, "GPS data struct",                                           //
                         A2L_MEASUREMENT_COMPONENT(latitude, "GPS latitude in degrees", ""),   //
                         A2L_MEASUREMENT_COMPONENT(longitude, "GPS longitude in degrees", ""), //
                         A2L_MEASUREMENT_COMPONENT(signal, "GPS signal quality", "")           //
        );

        // Create a typedef for struct ParametersT
        // (C style API)
        A2lTypedefBegin(ParametersT, &kParameters, "A2L Typedef for ParametersT");
        A2lTypedefParameterComponent(counter_max, "Maximum counter value", "", 0, 2000);
        A2lTypedefParameterComponent(delay_us, "Mainloop delay time in us", "us", 0, 999999);
        A2lTypedefParameterComponent(signal_amplitude, "Amplitude for the simulated GPS signal", "", 0.0, 10.0);
        A2lTypedefParameterComponent(use_simulated_time, "Whether to use simulated time for XCP events or real time", "", 0, 1);
        A2lTypedefEnd();

        // Add the calibration segment description as a typedef instance to the A2L file
        _parameters.CreateA2lTypedefInstance("ParametersT", "Main parameters");
    }

    // ----------------------------------------------------------------
    void InitControllers() override {}

    // ----------------------------------------------------------------
    void PublishGPSData(std::chrono::nanoseconds now) {

        // Demo measurement struct variable Subscriber._gps_data
        _gps_data.latitude = 48.8235;
        _gps_data.longitude = 9.0965;

        // Simulate a GPS signal strength as a sine wave between 0 and parameter Subscriber.signal_amplitude with a period of 1 second
        double time_sec = std::chrono::duration<double>(now).count();
        _gps_data.signal = _parameters.lock()->signal_amplitude * (0.5 * (std::sin(2.0 * M_PI * time_sec) + 1.0));

        // Publish the GPS data struct as a serialized byte array
        auto gpsSerialized = SerializeGPSData(_gps_data);
        _gpsPublisher->Publish(gpsSerialized);

        // printf("Published GPS data: lat=%g, lon=%g, signal=%g\n", _gps_data.latitude, _gps_data.longitude, _gps_data.signal);
    }

    // ----------------------------------------------------------------
    void PublishTemperatureData() {

        // Demo measurement variable Subscriber._temperature
        _temperature = 25.0 + static_cast<double>(rand() % 10) / 10.0;

        // Publish the temperature value as a serialized byte array
        auto temperatureSerialized = SerializeTemperature(_temperature);
        _temperaturePublisher->Publish(temperatureSerialized);

        // printf("Published temperature data: temperature=%g\n", _temperature);
    }

    // ----------------------------------------------------------------
    void DoWorkSync(std::chrono::nanoseconds now) override {

        // printf("Publisher: DoWorkSync %gs\n", now.count() * 1e-9);

        // Demo measurement variable Subscriber._counter
        // Limited by the calibration parameter Subscriber.counter_max
        _counter++;
        if (_counter > _parameters.lock()->counter_max)
            _counter = 0;

        PublishGPSData(now);
        PublishTemperatureData();

        // Trigger XCP measurement event DoWorkSync with simulated time
        // C++ variadic template style API, individual relative addressing mode for each measurement
        XcpUpdateSimTime(now.count());
        DaqEventVar(DoWorkSync,                                                               //
                    A2L_MEAS(_counter, "Simulation step counter"),                            //
                    A2L_MEAS_PHYS(_temperature, "Temperature in Celsius", "C", -50.0, 100.0), //
                    A2L_MEAS_INST(_gps_data, "GpsData", "GPS position data struct"));

        // Sleep some time to simulate work
        // Read delay_us before sleep to avoid holding the calibration lock during sleep
        const auto delay_us = _parameters.lock()->delay_us;
        if (delay_us > 0)
            std::this_thread::sleep_for(std::chrono::microseconds(delay_us));
    }

    // ----------------------------------------------------------------
    void DoWorkAsync() override {
        // printf("Publisher: DoWorkAsync\n");
        std::this_thread::sleep_for(std::chrono::microseconds(100));
    }
};

// ----------------------------------------------------------------
int main(int argc, char **argv) {

    // Initialize XCP server for measurement on TCP port 5555

    // Use this variant for a separate, dedicated XCP server participant
    // XcpServerInit("Publisher", "V1.7", 5555, XCP_MODE_SHM);

    // Use this variantfor a single XCP server participant
    // The first participant becomes the server, the others use shared memory
    XcpServerInit("Publisher", "V200", 5555, XCP_MODE_SHM_AUTO);

    Arguments args;
    args.participantName = "Publisher";
    Publisher app{args};
    app.SetupCommandLineArgs(argc, argv, "XCPlite SIL-Kit Demo - Publisher");
    return app.Run();
}
