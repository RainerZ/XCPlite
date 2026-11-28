#!/usr/bin/env python3
"""Sample windows and recordings: xcp_sample, xcp_measure_start, xcp_measure_stop, xcp_read and xcp_status while measuring."""

import time

from mcplib import *

target = Target()
mcp = Mcp(["--log-level", "3"])
try:
    mcp.initialize()
    mcp.call("xcp_connect", {"host": "127.0.0.1"}, contains="State: ready")

    print("\n===== sample")
    mcp.call(
        "xcp_sample",
        {"names": ["counter", "heat_power", "t1", "global_counter", "nosuch", "params.counter_max"], "duration_ms": 2000},
        contains=["Sampled 4 signals for 2.0 s", "counter  ", " samples, ", "  min ", "  mean ", "nosuch: not found.", "params.counter_max: is a calibration parameter without event"],
    )
    mcp.call("xcp_sample", {"names": ["nosuch"]}, ok=False, contains="nosuch: not found.")
    mcp.call("xcp_sample", {"names": []}, ok=False)

    # While a sample window is open: status and read are answered, the connection and the measurement are not changed
    started = time.time()
    sample = mcp.start_call("xcp_sample", {"names": ["counter"], "duration_ms": 3000})
    time.sleep(0.5)
    mcp.call("xcp_status", contains="State: ready")
    mcp.call("xcp_read", {"names": ["counter", "heat_power", "params.counter_max"]}, contains=["counter = ", "(measured, sample ", "heat_power = ", "(polled)", "(parameter, "])
    mcp.call("xcp_connect", {"host": "127.0.0.1", "port": 5556}, ok=False, contains="xcp_sample is running for another")
    mcp.call("xcp_measure_start", {"pattern": "."}, ok=False, contains="xcp_sample is running for another")
    mcp.call("xcp_sample", {"names": ["counter"]}, ok=False, contains="xcp_sample is already running")
    ok, text = mcp.result(sample)
    print(f"\n### the sample of 3 s, {time.time() - started:.1f} s after its start:\n{text}")
    check(ok and "Sampled 1 signal for 3.0 s" in text and 2.9 < time.time() - started < 4, "sample of 3 s")

    # A cancelled call closes the window
    sample = mcp.start_call("xcp_sample", {"names": ["counter"], "duration_ms": 20000})
    time.sleep(0.5)
    mcp.send({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": sample, "reason": "scenario"}})
    time.sleep(0.6)
    mcp.call("xcp_sample", {"names": ["counter"], "duration_ms": 300}, contains="Sampled 1 signal for 0.3 s")

    print("\n===== recording, default file")
    text = mcp.call("xcp_measure_start", {"pattern": "."}, contains=["Measurement started: 9 signals in 2 DAQ lists, until xcp_measure_stop.", "Files: hello_xcp_", ".mf4"])
    mdf = text.split("Files: ")[1].split()[0]
    time.sleep(1.5)
    mcp.call("xcp_status", contains=["State: measuring", "Measurement: 9 signals in 2 DAQ lists, running ", " events, ", "last sample "])
    mcp.call("xcp_read", {"names": ["counter", "heat_power", "params.counter_max"]}, contains=["counter = ", "heat_power = ", "(measured, sample "], lacks="(polled)")
    mcp.call("xcp_sample", {"names": ["counter", "heat_power"], "duration_ms": 1000}, contains="Sampled 2 signals for 1.0 s")
    mcp.call("xcp_measure_start", {"names": ["counter"]}, ok=False, contains="A measurement is already running")
    mcp.call("xcp_load_a2l", ok=False, contains="A measurement is running")
    # Calibration while measuring
    old = mcp.call("xcp_read", {"names": ["params.counter_max"]}).split(" = ")[1].split()[0]
    mcp.call("xcp_write", {"name": "params.counter_max", "value": 2000}, contains="-> 2000")
    mcp.call("xcp_write", {"name": "params.counter_max", "value": int(old)}, contains=f"2000 -> {old}")
    mcp.call("xcp_measure_stop", contains=["9 signals measured for ", "(stopped by xcp_measure_stop)", "0 packets lost", f"Files: {mdf} ("])
    mcp.call("xcp_status", contains=["State: ready", "Last measurement: 9 signals measured"])
    mcp.call("xcp_measure_stop", ok=False, contains="No measurement is running. The last one:")
    check(file_sizes().get(mdf, 0) > 1000, f"{mdf} written")

    print("\n===== recording with names, two files, duration 2 s")
    mcp.call(
        "xcp_measure_start",
        {"names": ["counter", "global_counter", "nosuch"], "csv": "m.csv", "mdf": "m.mf4", "duration_s": 2},
        contains=["Measurement started: 2 signals in 1 DAQ list, for 2 s, it stops by itself.", "Files: m.mf4, m.csv", "nosuch: not found."],
    )
    mcp.call("xcp_sample", {"names": ["counter", "heat_power"], "duration_ms": 500}, contains=["Sampled 1 signal", "heat_power: not part of the running measurement"])
    mcp.call("xcp_sample", {"names": ["heat_power"], "duration_ms": 500}, ok=False, contains="not part of the running measurement")
    time.sleep(2.0)
    mcp.call("xcp_status", contains=["State: ready", "Last measurement: 2 signals measured for 2.0 s (the requested duration is over)", "m.mf4 (", "m.csv ("])
    csv = (CLI / "m.csv").read_text().split("\n")
    print("m.csv:", csv[:2], "...", len(csv), "lines")
    check(csv[0] == "time_ns,daq,name,value,unit" and len(csv) > 1000, "m.csv has samples")

    print("\n===== refused recordings")
    mcp.call("xcp_measure_start", {"names": ["counter"], "csv": "m.csv"}, ok=False, contains="The file 'm.csv' exists, a measurement does not overwrite files.")
    mcp.call("xcp_measure_start", {"names": ["nosuch", "params.counter_max"]}, ok=False, contains="Nothing to measure.")
    mcp.call("xcp_measure_start", {"pattern": "zzz"}, ok=False, contains="No signals match 'zzz'")
    mcp.call("xcp_measure_start", {}, ok=False, contains="No signals given")
    mcp.call("xcp_measure_start", {"pattern": "("}, ok=False, contains="not a valid regular expression")
    mcp.call("xcp_measure_start", {"pattern": ".", "names": ["counter"]}, ok=False, contains="not both")

    print("\n===== MCAP and Perfetto")
    mcp.call("xcp_measure_start", {"pattern": "counter", "perfetto": "t.json", "mcap": "m.mcap", "duration_s": 1}, contains="Files: m.mcap, t.json")
    time.sleep(1.6)
    mcp.call("xcp_status", contains=["Last measurement: 2 signals measured for 1.0 s", "m.mcap (", "t.json ("])

    print("\n===== disconnect during a recording")
    mcp.call("xcp_measure_start", {"names": ["counter"], "mdf": "d.mf4"}, contains="1 signal in 1 DAQ list")
    time.sleep(0.5)
    mcp.call("xcp_disconnect", contains=["Disconnected from hello_xcp", "1 signal measured for ", "(stopped by xcp_disconnect)", "d.mf4 ("])
    mcp.call("xcp_status", contains=["State: disconnected", "Last measurement: 1 signal measured"])

    print("\n===== stdin closed during a recording and an open sample window")
    mcp.call("xcp_connect", {"host": "127.0.0.1"})
    mcp.call("xcp_measure_start", {"pattern": ".", "mdf": "e.mf4", "duration_s": 60})
    mcp.start_call("xcp_sample", {"names": ["counter"], "duration_ms": 20000})
    time.sleep(0.7)
    mcp.close()
    log = mcp.log_lines()
    print(*log[-3:], sep="\n")
    check(any("MDF file: " in line for line in log[-4:]), "the MDF file is completed at shutdown")
    check(log[-1].endswith("XCP Disconnected"), "DISCONNECT at shutdown")
    print("files:", file_sizes())
    for file in [mdf, "m.mf4", "m.csv", "m.mcap", "t.json", "d.mf4", "e.mf4"]:
        check(file_sizes().get(file, 0) > 0, f"{file} written")
finally:
    mcp.kill()
    target.kill()
finish()
