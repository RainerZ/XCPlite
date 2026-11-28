#!/usr/bin/env python3
"""Read only mode, the wrong protocol, an unreachable target, a host name, SIGTERM, an option which is not allowed with --mcp."""

import signal
import subprocess

from mcplib import *

target = Target()
mcp = None
try:
    print("===== read only, TCP as default protocol against a UDP target")
    mcp = Mcp(["--mcp-read-only", "--tcp", "--dest-addr", "127.0.0.1", "--log-level", "2"])
    mcp.initialize()
    tools = mcp.tool_names()
    print("tools:", tools)
    check("xcp_write" not in tools and "xcp_load_parameters" not in tools and len(tools) == 12, "read only: 12 tools without the writing ones")
    mcp.call("xcp_write", {"name": "params.counter_max", "value": 1}, ok=None)
    mcp.call("xcp_load_parameters", {"file": "p.hex"}, ok=None)
    mcp.call("xcp_status", contains="Default target of xcp_connect: 127.0.0.1 port 5555 (TCP)")
    mcp.call("xcp_connect", ok=False, contains=["Could not connect to 127.0.0.1:5555 (TCP)", "protocol (udp or tcp) right?"])
    mcp.call("xcp_connect", {"protocol": "udp", "host": "localhost"}, contains=["Connected.", "(UDP)"])
    mcp.call("xcp_load_a2l", {"default_event": "not an event"}, ok=False, contains="neither an event id")
    mcp.call("xcp_disconnect")

    print("\n===== unreachable target")
    mcp.call("xcp_connect", {"protocol": "udp", "host": "127.0.0.1", "port": 5999}, ok=False, contains="Could not connect to 127.0.0.1:5999 (UDP): Connect: Command response timeout")
    mcp.call("xcp_connect", {"protocol": "udp", "host": "no.such.host.invalid"}, ok=False, contains="Invalid target address")
    mcp.close()

    print("\n===== SIGTERM while connected")
    mcp = Mcp()
    mcp.initialize()
    mcp.call("xcp_connect", {"host": "127.0.0.1"})
    mcp.process.send_signal(signal.SIGTERM)
    check(mcp.process.wait(timeout=10) == 0, "exit status 0 after SIGTERM")
    log = mcp.log_lines()
    print(log[-2:])
    check(log[-1].endswith("XCP Disconnected"), "DISCONNECT after SIGTERM")

    print("\n===== an option which selects an action of the command line")
    result = subprocess.run([XCPCLIENT, "--mcp", "--mea", "counter"], stdin=subprocess.DEVNULL, capture_output=True, text=True)
    print(result.returncode, result.stderr.strip())
    check(result.returncode == 1 and result.stdout == "" and "--mea can not be used with --mcp" in result.stderr, "--mea with --mcp is rejected")
finally:
    if mcp:
        mcp.kill()
    target.kill()
finish()
