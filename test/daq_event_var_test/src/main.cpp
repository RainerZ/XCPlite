// daq_event_var_test - regression test for the variadic C++ event macros DaqEventVar, DaqEventAtVar and DaqTriggerEventVar
//
// The once state of these macros (event creation, A2L registration of the measurement variables) must be one per call site.
// Call sites with identical argument types instantiate the same template function, a static variable inside the template
// function is shared by all of them: the second call site would never register its measurements or would use the event of
// the first call site.
//
// Self-verifying and hermetic: no XCP server, no network. Generates daq_event_var_test.a2l in the working directory and
// checks that every measurement and every event of every call site has been registered.
// Returns 0 on success, non-zero on the first failed check.

#include <cstdint>  // for uintxx_t
#include <cstdio>   // for printf
#include <fstream>  // for std::ifstream
#include <iterator> // for std::istreambuf_iterator
#include <string>   // for std::string

#include "a2l.hpp"    // for A2l generation application programming interface
#include "xcplib.hpp" // for application programming interface

#include "../../support/check.h" // for the shared CHECK macro

static const char *kProjectName = "daq_event_var_test";
static const char *kA2lFileName = "daq_event_var_test.a2l";

static bool contains(const std::string &text, const char *pattern) { return text.find(pattern) != std::string::npos; }

int main() {

    XcpSetLogLevel(2);
    XcpInit(kProjectName, "V1.0", XCP_MODE_LOCAL);
    const uint8_t addr[4] = {127, 0, 0, 1};
    CHECK(A2lInit(addr, 5555, false, A2L_MODE_WRITE_ALWAYS), "A2lInit failed");

    // All call sites of a macro have identical argument types
    double a = 1.0, b = 2.0, c = 3.0, d = 4.0;
    struct BaseT {
        uint32_t x;
    } base_a{1}, base_b{2};

    // Execute every call site more than once
    for (int i = 0; i < 3; i++) {
        DaqEventVar(var_a, A2L_MEAS(a, "a"));
        DaqEventVar(var_b, A2L_MEAS(b, "b"));
        DaqEventAtVar(at_c, ApplXcpGetClock64(), A2L_MEAS(c, "c"));
        DaqEventAtVar(at_d, ApplXcpGetClock64(), A2L_MEAS(d, "d"));
        DaqTriggerEventVar(trg_a, base_a);
        DaqTriggerEventVar(trg_b, base_b);
    }

    // Every call site has its own event
    const char *event_names[] = {"var_a", "var_b", "at_c", "at_d", "trg_a", "trg_b"};
    const size_t event_count = sizeof(event_names) / sizeof(event_names[0]);
    tXcpEventId event_ids[event_count];
    for (size_t i = 0; i < event_count; i++) {
        event_ids[i] = XcpFindEvent(event_names[i]);
        CHECK(event_ids[i] != XCP_UNDEFINED_EVENT_ID, "event '%s' not created", event_names[i]);
        for (size_t j = 0; j < i; j++) {
            CHECK(event_ids[i] != event_ids[j], "events '%s' and '%s' share the event id %u", event_names[i], event_names[j], (unsigned)event_ids[i]);
        }
    }

    // Every call site has registered its measurement
    A2lFinalize();
    std::ifstream file(kA2lFileName);
    CHECK(file.is_open(), "A2L file '%s' not found", kA2lFileName);
    const std::string a2l((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    CHECK(contains(a2l, "/begin MEASUREMENT a "), "measurement 'a' of DaqEventVar call site 1 not registered");
    CHECK(contains(a2l, "/begin MEASUREMENT b "), "measurement 'b' of DaqEventVar call site 2 not registered");
    CHECK(contains(a2l, "/begin MEASUREMENT c "), "measurement 'c' of DaqEventAtVar call site 1 not registered");
    CHECK(contains(a2l, "/begin MEASUREMENT d "), "measurement 'd' of DaqEventAtVar call site 2 not registered");

    printf("daq_event_var_test PASSED\n");
    return 0;
}
