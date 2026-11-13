#!/usr/bin/env python3
"""Local dependency overrides preserve the tracked compiler configuration."""

from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest


class LocalCoreConfigTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.repo = self.root / "simulator"
        (self.repo / "scripts").mkdir(parents=True)
        (self.repo / ".cargo").mkdir()
        source = Path(__file__).resolve().parent.parent
        shutil.copy(source / "scripts/use-local-core.sh", self.repo / "scripts")
        self.config = self.repo / ".cargo/config.toml"
        shutil.copy(source / ".cargo/config.toml", self.config)
        self.config.write_text(self.config.read_text() + '\n[build]\ntarget-dir = "custom-target"\n')
        self.original = self.config.read_bytes()

    def core(self, path):
        path.mkdir(parents=True)
        (path / "Cargo.toml").write_text('[package]\nname = "xezim-core"\n')
        return path

    def run_helper(self, *arguments):
        result = subprocess.run(
            ["bash", str(self.repo / "scripts/use-local-core.sh"), *map(str, arguments)],
            cwd=self.repo,
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.config.read_bytes(), self.original)

    def test_explicit_path_with_spaces_and_repeated_removal(self):
        core = self.core(self.root / "external core")
        self.run_helper(core)
        override = self.repo / ".cargo/local-core.toml"
        patch = tomllib.loads(override.read_text())["patch"]["https://github.com/aionhw/xezim-core.git"]
        self.assertEqual(patch["xezim-core"]["path"], str(core))
        self.assertEqual(patch["sv-parser"]["path"], str(core / "xezim-parser"))
        self.run_helper("--remove")
        self.assertFalse(override.exists())
        self.run_helper("--remove")

    def test_nested_checkout_takes_precedence_over_sibling(self):
        self.core(self.repo / "xezim-core")
        self.core(self.root / "xezim-core")
        self.run_helper()
        override = self.repo / ".cargo/local-core.toml"
        patch = tomllib.loads(override.read_text())["patch"]["https://github.com/aionhw/xezim-core.git"]
        self.assertEqual(patch["xezim-core"]["path"], "./xezim-core")

    def test_reenabling_replaces_only_the_dependency_override(self):
        first = self.core(self.root / "first-core")
        second = self.core(self.root / "second-core")
        self.run_helper(first)
        self.run_helper(second)
        override = self.repo / ".cargo/local-core.toml"
        patch = tomllib.loads(override.read_text())["patch"]["https://github.com/aionhw/xezim-core.git"]
        self.assertEqual(patch["xezim-core"]["path"], str(second))


if __name__ == "__main__":
    unittest.main()
