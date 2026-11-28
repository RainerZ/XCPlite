#!/usr/bin/env python3
"""A2L and parameter files: xcp_create_a2l, xcp_load_a2l, xcp_save_parameters, xcp_load_parameters.

Uses the ELF fixtures of xcpclient and examples/no_a2l_demo/CANape/no_a2l_demo.elf, an ELF file with an EPK which
differs from the EPK of the target.
"""

import shutil
import subprocess

from mcplib import *

for elf in ["c_captures.elf", "c_meta_markers.elf", "cpp_types.elf"]:
    shutil.copy(FIXTURES / elf, CLI)
shutil.copy(REPO / "examples" / "no_a2l_demo" / "CANape" / "no_a2l_demo.elf", CLI)
(CLI / "out").mkdir(exist_ok=True)

target = Target()
mcp = Mcp(["--log-level", "2"])
try:
    mcp.initialize()

    print("===== create an A2L file offline, not connected")
    mcp.call("xcp_create_a2l", {"elf": "c_captures.elf", "a2l": "c_captures"}, contains=["A2L file c_captures.a2l created from c_captures.elf (offline, from the ELF file only)", "7 signals", "XCP_104.aml written", "variables not described"], lacks="Error:")
    mcp.call("xcp_create_a2l", {"elf": "c_captures.elf", "a2l": "c_captures.a2l"}, ok=False, contains="call xcp_create_a2l again with overwrite = true")
    mcp.call("xcp_create_a2l", {"elf": "c_captures.elf", "a2l": "c_captures.a2l", "overwrite": True, "default_event": "task"}, contains="created", lacks=["Overwriting", "XCP_104.aml written"])
    mcp.call("xcp_create_a2l", {"elf": "missing.elf", "a2l": "x.a2l"}, ok=False, contains="ELF file 'missing.elf' not found")
    mcp.call("xcp_create_a2l", {"elf": "cpp_types.elf", "a2l": "out/tpl.a2l", "template": True, "embed_aml": True}, contains="template without variables", lacks="XCP_104.aml written")
    mcp.call("xcp_create_a2l", {"elf": "c_meta_markers.elf", "a2l": "meta.a2l", "skip_no_metadata": True, "var_filter": "("}, ok=False, contains="regex")
    mcp.call("xcp_create_a2l", {"elf": "c_meta_markers.elf", "a2l": "meta.a2l", "skip_no_metadata": True}, contains="5 signals")
    mcp.call("xcp_create_a2l", {"elf": TARGET, "a2l": "target.a2l"}, ok=False, contains="could not be created") if sys.platform == "darwin" else None
    # A real application: hundreds of variables can not be described, the warnings are counted by reason
    text = mcp.call("xcp_create_a2l", {"elf": "no_a2l_demo.elf", "a2l": "out/offline.a2l"}, contains=["4 events, 3 calibration segments", "variables not described, location is a register", "out/XCP_104.aml written"])
    check(len(text.split("\n")) < 20, "the warnings of a real ELF file are summarized")
    mcp.call("xcp_load_a2l", ok=False, contains="Not connected")
    mcp.call("xcp_save_parameters", ok=False, contains="Not connected")

    print("\n===== connected: the EPK of the ELF file against the EPK of the target")
    mcp.call("xcp_connect", {"host": "127.0.0.1"}, contains="State: ready")
    mcp.call("xcp_create_a2l", {"elf": "c_captures.elf", "a2l": "out/online.a2l"}, contains=["with the events and calibration segments of hello_xcp", "The ELF file has no EPK"])
    mcp.call("xcp_create_a2l", {"elf": "c_captures.elf", "a2l": "out/offline2.a2l", "offline": True}, contains="offline, from the ELF file only")
    mcp.call("xcp_create_a2l", {"elf": "no_a2l_demo.elf", "a2l": "out/epk.a2l"}, ok=False, contains=["EPK mismatch: the ELF file has EPK", "call xcp_create_a2l again with accept_epk_mismatch = true"])
    check(not (CLI / "out" / "epk.a2l").exists(), "no A2L file after a refused EPK mismatch")
    mcp.call("xcp_create_a2l", {"elf": "no_a2l_demo.elf", "a2l": "out/epk.a2l", "accept_epk_mismatch": True}, contains="The EPK of the ELF file DIFFERS from the EPK of the target (accepted)")
    mcp.call("xcp_status", contains=["State: ready", "uploaded from the target"])

    print("\n===== load an A2L file")
    mcp.call("xcp_load_a2l", contains=["Description loaded.", "uploaded from the target"])
    a2l = uploaded_a2l()
    shutil.copy(a2l, CLI / "copy.a2l")
    shutil.copy(CACHE / "XCP_104.aml", CLI)
    epk = mcp.call("xcp_status").split("EPK: ")[1].split("\n")[0].split(",")[0]
    (CLI / "other.a2l").write_text(a2l.read_text().replace(epk, "V9.9"))
    mcp.call("xcp_load_a2l", {"a2l": "copy.a2l"}, contains=["copy.a2l (loaded from file)", "matches the A2L file"])
    mcp.call("xcp_load_a2l", {"a2l": "other.a2l"}, ok=False, contains=["EPK mismatch: the A2L file has EPK 'V9.9'", "call xcp_load_a2l again with accept_epk_mismatch = true", "copy.a2l (loaded from file)"])
    mcp.call("xcp_load_a2l", {"a2l": "other.a2l", "accept_epk_mismatch": True}, contains="DIFFERS from the EPK of the A2L file (accepted)")
    mcp.call("xcp_load_a2l", {"a2l": "copy.a2l", "fix": True}, ok=False, contains="was not created by xcpclient")
    mcp.call("xcp_load_a2l", {"a2l": "missing.a2l"}, ok=False, contains="Could not load A2L file 'missing.a2l'")
    mcp.call("xcp_load_a2l", {"elf": "no_a2l_demo.elf"}, ok=False, contains="EPK mismatch: the ELF file has EPK")
    mcp.call("xcp_connect", {"elf": "no_a2l_demo.elf"}, ok=False, contains="call xcp_connect again with accept_epk_mismatch = true")
    mcp.call("xcp_load_a2l", {"elf": "no_a2l_demo.elf", "a2l": "out/used.a2l", "accept_epk_mismatch": True}, contains=["out/used.a2l (created)", "DIFFERS from the EPK of the ELF file (accepted)"])
    mcp.call("xcp_load_a2l", contains="uploaded from the target")

    print("\n===== parameter files")
    mcp.call("xcp_list_parameters", contains="params.counter_max")
    text = mcp.call("xcp_save_parameters", contains=["2 calibration segments saved to hello_xcp_", ".hex ("])
    check(any(name.startswith("hello_xcp_") and name.endswith(".hex") for name in file_sizes()), "parameter file with the default name")
    mcp.call("xcp_save_parameters", {"file": "p.hex"}, contains="2 calibration segments saved to p.hex")
    mcp.call("xcp_save_parameters", {"file": "p.hex"}, ok=False, contains="a parameter file is not overwritten")
    mcp.call("xcp_save_parameters", {"file": "p.bin"}, contains="saved to p.bin")
    mcp.call("xcp_save_parameters", {"file": "p.txt"}, ok=False, contains="no known extension")
    # Change two parameters and restore them from the file
    counter_max, delay_us = [line.split(" = ")[1].split()[0] for line in mcp.call("xcp_read", {"names": ["params.counter_max", "params.delay_us"]}).split("\n")]
    mcp.call("xcp_write", {"name": "params.counter_max", "value": 500}, contains="-> 500")
    mcp.call("xcp_write", {"name": "params.delay_us", "value": 2000}, contains="-> 2000")
    mcp.call("xcp_load_parameters", {"file": "p.hex"}, contains="2 of 2 calibration segments loaded from p.hex")
    mcp.call("xcp_read", {"names": ["params.counter_max", "params.delay_us"]}, contains=[f"params.counter_max = {counter_max} ", f"params.delay_us = {delay_us} "])
    mcp.call("xcp_load_parameters", {"file": "p.bin"}, ok=False, contains="Only Intel-HEX files (.hex) can be loaded")
    mcp.call("xcp_load_parameters", {"file": "missing.hex"}, ok=False, contains="not found")
    (CLI / "empty.hex").write_text(":00000001FF\n")
    mcp.call("xcp_load_parameters", {"file": "empty.hex"}, ok=False, contains=["Nothing loaded", "No data found in IHEX file for segment params"])
    (CLI / "bad.hex").write_text("this is not a hex file\n")
    mcp.call("xcp_load_parameters", {"file": "bad.hex"}, ok=False, contains="could not be loaded")
    mcp.call("xcp_disconnect")
    mcp.close()

    print("\n===== command line: an ELF file with another EPK against the target")
    for args, status in [([], 1), (["--yes"], 0)]:
        result = subprocess.run([XCPCLIENT, "--udp", "--elf", "no_a2l_demo.elf", "--create-a2l", "--a2l", "out/cli.a2l"] + args, cwd=CLI, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        print(args, "-> exit status", result.returncode)
        check(result.returncode == status and "EPK mismatch: ELF file" in result.stdout, f"command line with {args}: exit status {status}")
    print("files:", file_sizes(), file_sizes(CLI / "out"))
finally:
    mcp.kill()
    target.kill()
finish()
