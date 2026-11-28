#!/usr/bin/env python3
"""The target dies during a recording: the files are completed, the state is disconnected.

Stops the target, no other XCP server may run on port 5555.
"""

import time

from mcplib import *

target = Target(own=True)
mcp = Mcp(["--log-level", "3"])
try:
    mcp.initialize()
    mcp.call("xcp_connect", {"host": "127.0.0.1"}, contains="State: ready")
    mcp.call("xcp_measure_start", {"names": ["counter"], "mdf": "k.mf4"}, contains="Measurement started: 1 signal in 1 DAQ list")
    time.sleep(1.0)
    target.kill()
    time.sleep(1.0)

    # Nothing notices as long as no command is sent: the last sample gets older
    mcp.call("xcp_read", {"names": ["counter"]}, contains=["counter = ", "(measured, sample 1 s old)"])
    mcp.call("xcp_status", contains=["State: measuring", "last sample "])

    # A command to the target notices it. The recording ends, its file is completed
    mcp.call(
        "xcp_read",
        {"names": ["counter", "params.counter_max"]},
        ok=False,
        contains=["Connection to hello_xcp at 127.0.0.1:5555 (UDP) lost", "1 signal measured for ", "(the connection was lost)", "k.mf4 ("],
    )
    mcp.call("xcp_status", contains=["State: disconnected", "Last measurement: 1 signal measured for ", "(the connection was lost)"])
    mcp.call("xcp_measure_stop", ok=False, contains="No measurement is running. The last one:")
    mcp.close()
    log = mcp.log_lines()
    check(any("MDF file: " in line and "records of 1 DAQ lists" in line for line in log), "the MDF file is completed")
    check(file_sizes().get("k.mf4", 0) > 1000, "k.mf4 written")
finally:
    mcp.kill()
    target.kill()
finish()
