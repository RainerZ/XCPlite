#!/usr/bin/env python3
"""Connect, status, lists, read, write, a lost connection, reconnect, shutdown.

Stops and restarts the target, no other XCP server may run on port 5555.
"""

from mcplib import *

target = Target(own=True)
mcp = Mcp()
try:
    info = mcp.initialize()
    check(info["serverInfo"]["name"] == "xcpclient", "server name")
    check("xcp_status" in info.get("instructions", ""), "instructions")
    tools = mcp.tool_names()
    print("tools:", tools)
    check(len(tools) == 14, f"14 tools expected, got {len(tools)}")

    # Not connected
    mcp.call("xcp_status", contains=["State: disconnected", "Default target of xcp_connect: 127.0.0.1 port 5555 (UDP)"])
    mcp.call("xcp_read", {"names": ["counter"]}, ok=False, contains="Not connected. Call xcp_connect first.")

    # Connect with A2L upload
    mcp.call("xcp_connect", {"host": "127.0.0.1", "port": 5555}, contains=["Connected.", "State: ready", "hello_xcp at 127.0.0.1:5555 (UDP)", "uploaded from the target"])
    mcp.call("xcp_status", contains="State: ready")
    mcp.call("xcp_connect", {"host": "127.0.0.1"}, contains="Already connected, nothing changed.")
    mcp.call("xcp_connect", {"host": "127.0.0.1", "port": 5556}, ok=False, contains=["Already connected to hello_xcp", "Call xcp_disconnect first"])
    mcp.call("xcp_connect", {"host": "localhost"}, contains="Already connected")

    # Lists
    mcp.call("xcp_list_parameters", contains=["3 of 3 parameters matching '.'", "params.counter_max  u16 = ", "limits 0..65535"])
    mcp.call("xcp_list_measurements", {"pattern": "counter|temp", "limit": 3}, contains=["3 of 5 signals", "next page: offset 3"])
    mcp.call("xcp_list_measurements", {"pattern": "counter|temp", "limit": 3, "offset": 3}, contains="2 of 5 signals")
    mcp.call("xcp_list_measurements", {"pattern": "("}, ok=False, contains="not a valid regular expression")

    # Read: parameters, global and stack variables by upload, a wrong name
    mcp.call(
        "xcp_read",
        {"names": ["params.counter_max", "params.flow_rate", "global_counter", "counter", "t1", "heat_power", "nosuch_counter"]},
        contains=["params.counter_max = ", "(parameter, limits 0..65535)", "global_counter = ", "t1 = ", "(polled)", "nosuch_counter: not found."],
    )
    mcp.call("xcp_read", {"names": ["nosuch"]}, ok=False, contains="not found")

    # Write with read back, limits, wrong targets
    old = mcp.call("xcp_read", {"names": ["params.counter_max"]}).split(" = ")[1].split()[0]
    mcp.call("xcp_write", {"name": "params.counter_max", "value": 500}, contains=f"params.counter_max: {old} -> 500")
    mcp.call("xcp_write", {"name": "params.counter_max", "value": 99999999}, ok=False, contains="exceeds the limits 0..65535, not written")
    mcp.call("xcp_write", {"name": "global_counter", "value": 1}, ok=False, contains="is a measurement variable")
    mcp.call("xcp_write", {"name": "counter_max", "value": 1}, ok=False, contains="Similar: params.counter_max")
    mcp.call("xcp_write", {"name": "params.flow_rate", "value": 0.5}, contains="-> 0.5")
    mcp.call("xcp_write", {"name": "params.flow_rate", "value": 0.301}, contains="0.5 -> 0.301")
    mcp.call("xcp_write", {"name": "params.counter_max", "value": int(old)}, contains=f"500 -> {old}")
    mcp.call("xcp_write", {"name": "params.flow_rate"}, ok=False, contains="value")
    mcp.call("no_such_tool", ok=None)

    # Concurrent calls are all answered
    ids = [mcp.start_call("xcp_read" if i % 2 else "xcp_status", {"names": ["global_counter"]} if i % 2 else {}) for i in range(6)]
    for id in reversed(ids):
        ok, text = mcp.result(id)
        check(ok, f"concurrent call {id}: {text}")

    # The target dies: the next command notices it, after two command timeouts
    print("\n===== target killed")
    target.kill()
    mcp.call("xcp_read", {"names": ["params.counter_max"]}, ok=False, contains=["Connection to hello_xcp at 127.0.0.1:5555 (UDP) lost", "Call xcp_connect to reconnect."])
    mcp.call("xcp_status", contains=["State: disconnected", "lost (ShortUpload: Command response timeout)"])
    mcp.call("xcp_list_parameters", ok=False, contains="Not connected")
    mcp.call("xcp_connect", ok=False, contains="Could not connect to 127.0.0.1:5555 (UDP)")

    print("\n===== target restarted")
    target.start()
    mcp.call("xcp_connect", contains=["Connected.", "State: ready"])
    mcp.call("xcp_read", {"names": ["params.counter_max"]}, contains="params.counter_max = ")
    mcp.call("xcp_disconnect", contains="Disconnected from hello_xcp")
    mcp.call("xcp_disconnect", contains="Not connected, nothing to do.")

    # The uploaded A2L file as a file given by name
    mcp.call("xcp_connect", {"a2l": str(uploaded_a2l())}, contains=["loaded from file", "matches the A2L file"])

    print("\n===== stdin closed")
    mcp.close()
    check(mcp.log_lines()[-1].endswith("XCP Disconnected"), "DISCONNECT at shutdown")
finally:
    mcp.kill()
    target.kill()
finish()
