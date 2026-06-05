# Clock synchronisation test - clock_test


XCP test client with simulated drifting and jittery clock.  

Uses the generic clock synchronizer (see util.h/c) to generate an XCP time scale from the system monotonic clock or PTP clock (#define OPTION_ENABLE_PTP), and allows to adjust the offset, drift, drift of drift and jitter of this generated clock via calibration parameters.


Provides an approximately 10ms event in system time scale and a precise pps event in XCP time scale, together with some measurement signals.  


Can be used to test stability and tolerance of XCP clients against clock offset jumps, drifts and jitter.  
See the example CANape project in test/clock_test/CANape.  

 


