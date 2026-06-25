// silkit_demo - Subscriber application

#include "ApplicationBase.hpp"
#include "PubSubDemoCommon.hpp"
#include "XcpHelper.hpp"

using namespace PubSubDemoCommon;

class Subscriber : public ApplicationBase {
  public:
    // Inherit constructors
    using ApplicationBase::ApplicationBase;

    ~Subscriber() { XcpServerShutdown(); }

  private:
    IDataSubscriber *_gpsSubscriber;
    IDataSubscriber *_temperatureSubscriber;

    uint16_t _counter = 0;
    double _signal = 0.0;
    GpsData _gps_data = {0.0, 0.0, 0.0};
    double _temperature = 0.0;

    void AddCommandLineArgs() override {}

    void EvaluateCommandLineArgs() override {}

    // ----------------------------------------------------------------
    void CreateControllers() override {

        // Create data subscribers for GPS and temperature data

        _gpsSubscriber = GetParticipant()->CreateDataSubscriber( //
            "GpsSubscriber",                                     //
            dataSpecGps,                                         //

            [this](IDataSubscriber * /*subscriber*/, const DataMessageEvent &dataMessageEvent) {
                // Deserialize the received GPS data struct from the serialized byte array
                _gps_data = DeserializeGPSData(SilKit::Util::ToStdVector(dataMessageEvent.data));

                // Trigger XCP measurement event Subscriber.Gps with the timestamp of the received data message
                // @@@@ TODO: Use the current time instead
                XcpUpdateSimTime(dataMessageEvent.timestamp.count());
                DaqTriggerEventExt(Gps, this);
            });

        _temperatureSubscriber = GetParticipant()->CreateDataSubscriber( //
            "TemperatureSubscriber",                                     //
            dataSpecTemperature,                                         //

            [this](IDataSubscriber * /*subscriber*/, const DataMessageEvent &dataMessageEvent) {
                // Deserialize the received temperature value from the serialized byte array
                _temperature = DeserializeTemperature(SilKit::Util::ToStdVector(dataMessageEvent.data));
                // printf("Subscriber: Received temperature data: temperature=%g\n", _temperature);

                // Trigger XCP measurement event Subscriber.Temp with the timestamp of the received data message
                // @@@@ TODO: Use the current time instead
                XcpUpdateSimTime(dataMessageEvent.timestamp.count());
                DaqTriggerEventExt(Temp, this);
            });

        // Create a typedef for struct GpsData
        A2lCreateTypedef(GpsData, "GPS data struct", A2L_MEASUREMENT_COMPONENT(latitude, "GPS latitude in degrees", ""), //
                         A2L_MEASUREMENT_COMPONENT(longitude, "GPS longitude in degrees", ""),                           //
                         A2L_MEASUREMENT_COMPONENT(signal, "GPS signal quality", "")                                     //
        );

        // Create events and associate measurements of instance variables (addressing mode relative to this)

        DaqCreateEvent(DoWorkSync); // On simulation step
        A2lSetRelativeAddrMode(DoWorkSync, this);
        A2lCreateMeasurement(_counter, "Simulation step counter"); // Declare and associate _counter with the DoWorkSync event
        A2lCreateMeasurement(_signal, "GPS signal strength");      // Declare and associate _signal with the GPS signal strength event

        DaqCreateEvent(Gps); // On reception callback for of GPS data
        A2lSetRelativeAddrMode(Gps, this);
        A2lCreateTypedefInstance(_gps_data, GpsData, "GPS data struct"); // Declare and associate _gps_data with the GpsData event

        DaqCreateEvent(Temp); // On reception callback for temperature data
        A2lSetRelativeAddrMode(Temp, this);
        A2lCreateMeasurement(_temperature, "Received temperature in Celsius"); // Declare and associate _temperature with the Temp event
    }

    // ----------------------------------------------------------------
    void InitControllers() override {}

    // ----------------------------------------------------------------
    void DoWorkSync(std::chrono::nanoseconds now) override {

        // printf("Subscriber: DoWorkSync %gs\n", now.count() * 1e-9);

        // Demo measurement variable Subscriber._counter
        _counter++;
        if (_counter > 1000)
            _counter = 0;

        // Demo measurement variable Subscriber._signal (just a copy of the GPS signal strength)
        _signal = _gps_data.signal;

        // Trigger XCP measurement event Subscriber.DoWorkSync with simulated time
        // C style API, global, stack relative and this relative addressing mode
        XcpUpdateSimTime(now.count());
        DaqTriggerEventExt(DoWorkSync, this);

        // Sleep some time to simulate work
        std::this_thread::sleep_for(std::chrono::microseconds(100));
    }

    // ----------------------------------------------------------------
    void DoWorkAsync() override {

        // printf("Subscriber: DoWorkAsync\n");
        std::this_thread::sleep_for(std::chrono::microseconds(100));
    }
};

// ----------------------------------------------------------------
int main(int argc, char **argv) {

    // Initialize XCP server

    // Use this variant for a separate, dedicated XCP server participant
    // XcpServerInit("Publisher", "V1.7", 5555, XCP_MODE_SHM);

    // Use this variant for multi-application shared memory mode
    // The first participant becomes the server, the others use shared memory
    XcpServerInit("Subscriber", "V200", 5555, XCP_MODE_SHM_AUTO);

    Arguments args;
    args.participantName = "Subscriber";
    Subscriber app{args};
    app.SetupCommandLineArgs(argc, argv, "XCPlite SIL-Kit Demo - Subscriber");

    return app.Run();
}
