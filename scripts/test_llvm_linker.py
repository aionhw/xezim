#!/usr/bin/env python3
"""The Clang linker driver keeps LLD selected and preserves linker arguments."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class LlvmLinkerTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.libdir = self.root / "toolchain/lib"
        self.libdir.mkdir(parents=True)
        self.record = self.root / "arguments"
        self.rustc = self.bin / "test-rustc"
        self.rustc.write_text('#!/bin/sh\nprintf "%s\\n" "$LINKER_TEST_LIBDIR"\n')
        self.rustc.chmod(0o755)
        clang = self.bin / "clang"
        clang.write_text(
            '#!/bin/sh\nprintf "%s\\0" "$@" > "$LINKER_TEST_RECORD"\n'
            'exit "${LINKER_TEST_STATUS:-0}"\n'
        )
        clang.chmod(0o755)
        self.driver = Path(__file__).resolve().parent.parent / ".cargo/clang-linker.sh"

    def invoke(self, status=0):
        arguments = ["-o", "output with spaces", "input.o"]
        result = subprocess.run(
            ["bash", str(self.driver), *arguments],
            env=os.environ | {
                "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                "RUSTC": str(self.rustc),
                "RUSTFLAGS": "-C target-cpu=native",
                "LINKER_TEST_LIBDIR": str(self.libdir),
                "LINKER_TEST_RECORD": str(self.record),
                "LINKER_TEST_STATUS": str(status),
            },
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, status, result.stdout + result.stderr)
        actual = self.record.read_bytes().decode().split("\0")[:-1]
        self.assertEqual(actual[1:], arguments)
        return actual[0]

    def test_uses_the_selected_rust_toolchains_lld(self):
        linker = self.libdir.parent / "bin/gcc-ld/ld.lld"
        linker.parent.mkdir(parents=True)
        linker.write_text("#!/bin/sh\nexit 0\n")
        linker.chmod(0o755)
        self.assertEqual(self.invoke(), "-fuse-ld=" + str(linker))

    def test_uses_system_lld_when_the_toolchain_has_no_linker(self):
        self.assertEqual(self.invoke(), "-fuse-ld=lld")

    def test_propagates_the_compiler_exit_status(self):
        self.assertEqual(self.invoke(status=9), "-fuse-ld=lld")


if __name__ == "__main__":
    unittest.main()
