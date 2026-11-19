#!/usr/bin/env python3
"""Synthesize VHDL RTL with GHDL and simulate the resulting netlist with xezim."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile


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
            [args.xezim, "-s", args.top, *argv[sep + 1:], netlist],
            env={**os.environ, "XEZIM_JIT": "1"},
        ).returncode


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except OSError as error:
        sys.exit(str(error))
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
