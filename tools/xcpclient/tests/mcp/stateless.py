#!/usr/bin/env python3
"""Protocol revision 2026-07-28: no initialize handshake, every request carries the protocol version."""

from mcplib import *

META = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientInfo": {"name": "scenario", "version": "0"},
    "io.modelcontextprotocol/clientCapabilities": {},
}

target = Target()
mcp = Mcp()
try:
    response = mcp.request("server/discover", {"_meta": META})["result"]
    print("server/discover:", response["supportedVersions"], list(response["capabilities"]))
    check("2026-07-28" in response["supportedVersions"], "revision 2026-07-28 supported")
    check("xcp_status" in response.get("instructions", ""), "instructions")

    response = mcp.request("tools/list", {"_meta": META})["result"]
    print("tools/list:", len(response["tools"]), "tools,", {k: v for k, v in response.items() if k != "tools"})
    check(len(response["tools"]) == 14 and response.get("resultType") == "complete", "tool list")

    for tool, args, is_error, text in [
        ("xcp_status", {}, False, "State: disconnected"),
        ("xcp_connect", {"host": "127.0.0.1"}, False, "Connected."),
        ("xcp_read", {"names": ["params.counter_max", "nosuch"]}, False, "params.counter_max = "),
        ("xcp_read", {"names": ["nosuch"]}, True, "not found"),
    ]:
        result = mcp.request("tools/call", {"name": tool, "arguments": args, "_meta": META})["result"]
        first_line = result["content"][0]["text"].split("\n")[0]
        print(tool, "->", {k: v for k, v in result.items() if k != "content"}, "|", first_line)
        check(result.get("resultType") == "complete" and bool(result.get("isError")) == is_error and text in result["content"][0]["text"], f"{tool} {args}")
    mcp.close()
finally:
    mcp.kill()
    target.kill()
finish()
