"""Exercise the synthesized VHDL runner against actual GHDL output."""

from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(shutil.which("ghdl"), "GHDL is required")
class SynthesizedTopTests(unittest.TestCase):
    def test_entity_spelling(self):
        runner = Path(__file__).with_name("run-vhdl-rtl.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "top.vhd"
            source.write_text(
                "entity MixedCase is port (a : in bit; z : out bit); end;\n"
                "architecture rtl of MixedCase is begin z <= a; end;\n"
            )
            simulator = root / "xezim"
            simulator.write_text(
                "#!/usr/bin/env python3\n"
                "import os, pathlib, re, sys\n"
                "assert sys.argv[1] == '-s'\n"
                "assert os.environ['XEZIM_JIT'] == '1'\n"
                "assert sys.argv[3] == '--no-cache'\n"
                "netlist = pathlib.Path(sys.argv[-1]).read_text()\n"
                "assert re.search(r'^module\\s+' + re.escape(sys.argv[2]) + r'\\b', netlist, re.M)\n"
            )
            simulator.chmod(0o755)
            for spelling in ("MixedCase", "mixedcase", "MIXEDCASE"):
                with self.subTest(spelling=spelling):
                    result = subprocess.run(
                        [sys.executable, str(runner), "--top", spelling,
                         "--xezim", str(simulator), str(source), "--", "--no-cache"],
                        text=True, capture_output=True,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
