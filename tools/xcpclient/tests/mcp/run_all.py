#!/usr/bin/env python3
"""Run all MCP scenarios of this directory, each in its own working directory. Exit status 1 if one fails.

No XCP server may run on port 5555: some scenarios stop and restart the target.
Usage: run_all.py [scenario ...]
"""

import subprocess
import sys
import time
from pathlib import Path

SCENARIOS = ["basic", "errors", "stateless", "measure", "measure_lost", "files", "same_directory"]

directory = Path(__file__).resolve().parent
failed = []
for scenario in sys.argv[1:] or SCENARIOS:
    started = time.time()
    result = subprocess.run([sys.executable, str(directory / f"{scenario}.py")], capture_output=True, text=True)
    output = result.stdout + result.stderr
    print(f"{scenario:16} {'ok' if result.returncode == 0 else 'FAILED':7} {time.time() - started:5.1f} s")
    if result.returncode != 0:
        failed.append(scenario)
        # The failed expectations, or the end of the output of a scenario which did not get that far
        lines = [line for line in output.split("\n") if "FAILED" in line or line.startswith("  ")]
        print("\n".join("    " + line for line in (lines or output.strip().split("\n")[-8:])))
if failed:
    sys.exit(f"{len(failed)} of {len(sys.argv[1:] or SCENARIOS)} scenarios failed: {', '.join(failed)}")
print("All scenarios passed")
