#!/usr/bin/env python3
"""xcpclient in the working directory of the target: the A2L file of the target keeps its content.

Regression test: an XCPlite application writes its A2L file <name>.a2l into its working directory when a client connects,
and reads it for the upload. A client in the same directory which uploads to the same name emptied this file. The MCP server
is started in the project directory by its host, which is the directory an example is run in.

Starts the target in the directory of xcpclient, no other XCP server may run on port 5555.
"""

import hashlib
import subprocess
import time

from mcplib import *

if port_in_use():
    sys.exit(f"This scenario starts its target in the directory of xcpclient, but an XCP server is already running on port {XCP_PORT}. Stop it first.")


def a2l_state(directory):
    """Name, size and checksum of the A2L file of the target in a directory"""
    files = [f for f in sorted(directory.glob("*.a2l")) if not f.name.endswith("_upload.a2l")]
    if not files:
        return None
    data = files[0].read_bytes()
    return files[0].name, len(data), hashlib.md5(data).hexdigest()[:8]


def run(name, args, mcp_dir):
    """Target and MCP server in one directory: connect twice, the A2L file of the target must not change"""
    print(f"\n===== {name}")
    directory = WORK / name
    directory.mkdir()
    target = subprocess.Popen([TARGET], cwd=directory, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1.5)
    mcp = Mcp(args, cwd=directory, mcp_dir=mcp_dir)
    try:
        mcp.initialize()
        mcp.call("xcp_connect", contains=["Connected.", "State: ready"])
        first = a2l_state(directory)
        print("A2L file of the target after the connect:", first)
        check(first is not None and first[1] > 1000, "the target has written its A2L file and it has content")
        mcp.call("xcp_list_measurements", {"limit": 3}, contains="signals matching")
        mcp.call("xcp_disconnect")
        mcp.call("xcp_connect", contains="State: ready")
        second = a2l_state(directory)
        print("A2L file of the target after the second connect:", second)
        check(second == first, "the A2L file of the target is unchanged")
        mcp.close()
        print("files in the directory:", sorted(f.name for f in directory.iterdir()))
    finally:
        mcp.kill()
        target.kill()
        target.wait()
    return directory, first


# The files of the server go to its own directory, by default the cache directory of the user: here a directory of the run
directory, _ = run("own_directory", [], str(WORK / "cache"))
check(not (directory / "XCP_104.aml").exists(), "nothing but the files of the target in its directory")

# --mcp-dir set to the directory of the target: the upload replaces the A2L file by an identical copy
directory, state = run("same_directory", [], ".")

# The command line in the directory of the target: the upload goes to <name>_upload.a2l
print("\n===== command line")
directory = WORK / "command_line"
directory.mkdir()
(directory / "XCP_104.aml").write_bytes((XCPCLIENT_DIR / "XCP_104.aml").read_bytes())
target = subprocess.Popen([TARGET], cwd=directory, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1.5)
try:
    for i in range(2):
        result = subprocess.run([XCPCLIENT, "--udp", "--upload-a2l", "--list-mea", "counter"], cwd=directory, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        check(result.returncode == 0 and "already exists and has the A2L file name of the XCP server, uploading to" in result.stdout, f"upload {i + 1} goes to another file")
    name, size, checksum = a2l_state(directory)
    upload = directory / name.replace(".a2l", "_upload.a2l")
    print("A2L file of the target:", (name, size, checksum), "upload:", upload.name, upload.exists())
    check(size > 1000 and upload.exists() and upload.read_bytes() == (directory / name).read_bytes(), "the upload is a copy, the A2L file of the target has its content")
finally:
    target.kill()
    target.wait()
finish()
