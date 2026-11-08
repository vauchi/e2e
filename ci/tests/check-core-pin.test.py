#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

"""Pin validation separates wrong releases from failed manifest searches."""

import os
import shlex
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "check-core-pin.sh"
GREP = shutil.which("grep")
SH = shutil.which("sh")


class CorePinGuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.manifest = self.root / "Cargo.toml"
        self.write_tag("v1.2.3")
        self.bin = self.root / "bin"
        self.bin.mkdir()

    def write_tag(self, tag):
        self.manifest.write_text('vauchi-core = { git = "https://gitlab.com/vauchi/core.git", '
                                 f'tag = "{tag}" }}\n')

    def run_guard(self, path=None, manifest=None):
        env = dict(os.environ)
        if path is not None:
            env["PATH"] = path
        return subprocess.run([SH, str(SCRIPT), str(manifest or self.manifest), "v1.2.3"],
                              env=env, capture_output=True, text=True)

    def test_matching_release_succeeds(self):
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("OK:", result.stdout)

    def test_different_release_is_rejected(self):
        self.write_tag("v1.2.4")
        result = self.run_guard()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("another tag", result.stderr)

    def test_release_tag_is_literal_not_a_regular_expression(self):
        self.write_tag("v1x2y3")
        result = self.run_guard()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertNotIn("OK:", result.stdout)

    def test_manifest_without_core_git_dependency_is_rejected(self):
        self.manifest.write_text('vauchi-core = { path = "../core" }\n')
        result = self.run_guard()
        self.assertEqual(result.returncode, 1)
        self.assertIn("takes no Core crate", result.stderr)

    def test_unreadable_manifest_is_an_operational_error(self):
        result = self.run_guard(manifest=self.root / "missing.toml")
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("OK:", result.stdout)

    def test_backend_failure_in_either_search_is_not_clean_validation(self):
        self.write_tag("v1.2.4")
        for phase in ["first", "comparison"]:
            with self.subTest(phase=phase):
                stub = self.bin / "grep"
                stub.write_text("#!/bin/sh\nfail=" + phase + "\n"
                                'for arg do [ "$arg" != "-v" ] || fail=first; done\n'
                                'if [ "$fail" = first ]; then\n'
                                "  echo 'injected search failure' >&2\n  exit 2\nfi\n"
                                "exec " + shlex.quote(GREP) + ' "$@"\n')
                stub.chmod(0o755)
                result = self.run_guard(path=str(self.bin) + os.pathsep + os.environ["PATH"])
                self.assertEqual(result.returncode, 2)
                self.assertIn("injected search failure", result.stderr)
                self.assertNotIn("OK:", result.stdout)

    def test_missing_search_tool_is_an_error(self):
        result = self.run_guard(path=str(self.bin))
        self.assertEqual(result.returncode, 127)
        self.assertNotIn("OK:", result.stdout)


if __name__ == "__main__":
    unittest.main()
