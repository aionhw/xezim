#!/usr/bin/env python3
"""Synthesize VHDL RTL with GHDL and simulate the resulting netlist with xezim."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def verilog_top(netlist, requested):
    # VHDL basic identifiers are case-insensitive; Verilog identifiers are not.
    # GHDL preserves the entity declaration's spelling in its output.
    matches = set()
    module_header = re.compile(r"^\s*module\s+([^\s(;]+)")
    with netlist.open() as source:
        for line in source:
            match = module_header.match(line)
            if match:
                name = match[1]
                if name == requested:
                    return name
                if name.casefold() == requested.casefold():
                    matches.add(name)
    if len(matches) == 1:
        return matches.pop()
    raise ValueError(f"Cannot identify synthesized Verilog top for {requested!r}")


def main(argv):
    sep = argv.index("--") if "--" in argv else len(argv)
    parser = argparse.ArgumentParser(
        description=__doc__,
        epilog="Put xezim simulation options after --. Sources are in dependency order.",
    )
    parser.add_argument("--top", required=True, help="VHDL top entity")
    parser.add_argument("--ghdl", default="ghdl", help="GHDL synthesis executable")
    parser.add_argument("--xezim", default="xezim", help="xezim executable built with --features jit")
    parser.add_argument("sources", nargs="+", type=Path, help="VHDL-2008 sources in library work")
    args = parser.parse_args(argv[:sep])
    with tempfile.TemporaryDirectory(prefix="xezim-vhdl-rtl-") as directory:
        netlist = Path(directory) / "design.v"
        with netlist.open("wb") as output:
            subprocess.run(
                [args.ghdl, "synth", "--std=08", "--out=verilog", *args.sources, "-e", args.top],
                stdout=output,
                check=True,
            )
        return subprocess.run(
            [args.xezim, "-s", verilog_top(netlist, args.top), *argv[sep + 1:], netlist],
            env={**os.environ, "XEZIM_JIT": "1"},
        ).returncode


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (OSError, ValueError) as error:
        sys.exit(str(error))
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
