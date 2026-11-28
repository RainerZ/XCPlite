"""Helpers for the MCP scenario scripts in this directory, see README.md.

A scenario starts xcpclient --mcp as a child process, sends MCP messages to its stdin and checks the tool results.
The XCP target is build/hello_xcp of the repository.
"""

import json
import os
import select
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
XCPCLIENT_DIR = REPO / "tools" / "xcpclient"
FIXTURES = XCPCLIENT_DIR / "fixtures"
XCP_PORT = 5555


def _xcpclient():
    """The xcpclient to test: $XCPCLIENT, or the newer of the release and the debug build"""
    if os.environ.get("XCPCLIENT"):
        return os.environ["XCPCLIENT"]
    builds = [XCPCLIENT_DIR / "target" / build / "xcpclient" for build in ("release", "debug")]
    builds = [build for build in builds if build.exists()]
    if not builds:
        sys.exit("xcpclient is not built: run 'cargo build' in tools/xcpclient, or set XCPCLIENT")
    return str(max(builds, key=lambda build: build.stat().st_mtime))


XCPCLIENT = _xcpclient()
TARGET = os.environ.get("XCP_TARGET", str(REPO / "build" / "hello_xcp"))

# Working directory of a run: the target runs in srv, xcpclient in cli, its own files go to cli/cache (--mcp-dir)
WORK = Path(os.environ.get("XCP_MCP_WORK") or tempfile.mkdtemp(prefix="xcpclient-mcp-"))
SRV = WORK / "srv"
CLI = WORK / "cli"
CACHE = CLI / "cache"
for directory in (SRV, CLI):
    directory.mkdir(parents=True, exist_ok=True)

_failures = []


def check(condition, what):
    """Count a failed expectation and go on, the scenario ends with finish()"""
    if not condition:
        _failures.append(what)
        print(f"!!! FAILED: {what}")
    return condition


def finish():
    """End of a scenario: print the summary and exit with status 1 if an expectation failed"""
    print(f"\nxcpclient: {XCPCLIENT}\nworking directory: {WORK}")
    if _failures:
        print(f"\n{len(_failures)} FAILED:")
        for failure in _failures:
            print("  " + failure)
        sys.exit(1)
    print("\nOK")


def port_in_use():
    """An XCP server is running on the XCP port (UDP)"""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.bind(("0.0.0.0", XCP_PORT))
        return False
    except OSError:
        return True
    finally:
        s.close()


class Target:
    """The XCP target of a scenario: hello_xcp started in its own directory, or an XCP server which is already running.

    own=True: the scenario stops and restarts the target, a server which is already running is an error.
    """

    def __init__(self, own=False):
        self.process = None
        if port_in_use():
            if own:
                sys.exit(f"This scenario stops and restarts its target, but an XCP server is already running on port {XCP_PORT}. Stop it first.")
            print(f"Using the XCP server which is already running on port {XCP_PORT}")
        else:
            self.start()

    def start(self):
        if not Path(TARGET).exists():
            sys.exit(f"Target {TARGET} not found: build the examples (./build.sh), or set XCP_TARGET")
        self.process = subprocess.Popen([TARGET], cwd=SRV, stdout=open(SRV / "target.log", "a"), stderr=subprocess.STDOUT)
        time.sleep(1.5)

    def kill(self):
        if self.process:
            self.process.kill()
            self.process.wait()
            self.process = None


class Mcp:
    """xcpclient --mcp as a child process, with MCP messages over its stdin and stdout"""

    def __init__(self, args=(), cwd=CLI, mcp_dir="cache"):
        self.cwd = Path(cwd)
        self.log = self.cwd / "xcpclient.log"
        command = [XCPCLIENT, "--mcp"] + (["--mcp-dir", mcp_dir] if mcp_dir else []) + list(args)
        self.process = subprocess.Popen(command, cwd=self.cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(self.log, "w"))
        self.id = 0
        self.pending = {}
        self.stray = 0

    def send(self, message):
        self.process.stdin.write((json.dumps(message) + "\n").encode())
        self.process.stdin.flush()

    def recv(self, id, timeout=40):
        """The response to the request id. Every line on stdout must be an MCP message"""
        if id in self.pending:
            return self.pending.pop(id)
        end = time.time() + timeout
        while time.time() < end:
            readable, _, _ = select.select([self.process.stdout], [], [], 0.2)
            if not readable:
                continue
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError("stdout of xcpclient closed")
            try:
                message = json.loads(line)
            except ValueError:
                self.stray += 1
                check(False, f"not an MCP message on stdout: {line!r}")
                continue
            if message.get("id") == id:
                return message
            self.pending[message.get("id")] = message
        raise TimeoutError(f"no response to request {id}")

    def request(self, method, params=None, timeout=40):
        self.id += 1
        message = {"jsonrpc": "2.0", "id": self.id, "method": method}
        if params is not None:
            message["params"] = params
        self.send(message)
        return self.recv(self.id, timeout)

    def initialize(self):
        """The handshake of the protocol revisions up to 2025-11-25"""
        response = self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "scenario", "version": "0"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        return response["result"]

    def tool_names(self):
        return sorted(tool["name"] for tool in self.request("tools/list")["result"]["tools"])

    def start_call(self, tool, args=None):
        """Send a tool call without waiting for its result, returns the request id for result()"""
        self.id += 1
        self.send({"jsonrpc": "2.0", "id": self.id, "method": "tools/call", "params": {"name": tool, "arguments": args or {}}})
        return self.id

    def result(self, id, timeout=40):
        """The result of a tool call: (ok, text). ok is None for a protocol error"""
        response = self.recv(id, timeout)
        if "error" in response:
            return None, json.dumps(response["error"])
        result = response["result"]
        return not result.get("isError"), "\n".join(content.get("text", "") for content in result.get("content", []))

    def call(self, tool, args=None, ok=True, contains=(), lacks=(), timeout=40):
        """Call a tool, print the result and check it: ok (True, False for a tool error, None for a protocol error),
        texts the result contains and texts it must not contain. Returns the text"""
        started = time.time()
        is_ok, text = self.result(self.start_call(tool, args), timeout)
        state = {True: "ok", False: "ERROR", None: "PROTOCOL ERROR"}[is_ok]
        print(f"\n### {tool} {json.dumps(args or {})}  -> {state} ({time.time() - started:.1f}s)\n{text}")
        what = f"{tool} {json.dumps(args or {})}"
        check(is_ok == ok, f"{what}: expected {'a result' if ok else 'an error'}, got {state}")
        for expected in [contains] if isinstance(contains, str) else contains:
            check(expected in text, f"{what}: result does not contain {expected!r}")
        for unexpected in [lacks] if isinstance(lacks, str) else lacks:
            check(unexpected not in text, f"{what}: result contains {unexpected!r}")
        return text

    def close(self, timeout=15):
        """Close stdin: xcpclient stops a measurement, disconnects and ends with exit status 0"""
        self.process.stdin.close()
        status = self.process.wait(timeout=timeout)
        check(status == 0, f"exit status of xcpclient is {status}")
        check(self.process.stdout.read() == b"", "output on stdout after the last response")
        return status

    def log_lines(self):
        return self.log.read_text().strip().split("\n")

    def kill(self):
        if self.process.poll() is None:
            self.process.kill()


def uploaded_a2l():
    """The A2L file xcpclient has uploaded from the target into its own directory"""
    files = sorted(CACHE.glob("*.a2l"))
    check(len(files) == 1, f"one uploaded A2L file expected in {CACHE}, found {[f.name for f in files]}")
    return files[0]


def file_sizes(directory=CLI):
    return {f.name: f.stat().st_size for f in sorted(Path(directory).iterdir()) if f.is_file()}
