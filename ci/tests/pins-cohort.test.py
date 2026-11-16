# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

# @internal — real Git and HTTP boundaries, synthetic credentials only.
from pathlib import Path
import unittest

from pins_cohort_fixture import BRANCH, FILES, NEW, OLD, CohortFixture

ENTRY = Path(__file__).resolve().parents[1] / "pins-cohort.sh"


class CohortTests(unittest.TestCase):
    def fixture(self, **kwargs):
        fixture = CohortFixture(ENTRY, **kwargs)
        self.addCleanup(fixture.close)
        return fixture

    def succeeds(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_branch_code_never_receives_the_write_credential(self):
        fixture = self.fixture()
        fixture.change({"ci/update-pins.sh": '#!/bin/sh\nprintf branch-code-ran > "$COHORT_BRANCH_SENTINEL"\nexit 2\n',
                        ".cargo/config.toml": "# BRANCH_CONTROLLED\n"})
        self.succeeds(fixture.run())
        self.assertFalse(fixture.sentinel.exists(), "branch updater executed with the inherited write credential")
        self.assertIn(f"RELAY_SHA={NEW}", fixture.read("ci/pins.env"))
        self.assertEqual(fixture.read("unrelated.rs"), "preserve branch Rust fixes")
        self.assertEqual(len(fixture.posts), 1)
        self.assertEqual({a["file_path"] for a in fixture.posts[0]["actions"]}, set(FILES))

    def test_no_open_cohort_does_not_run_cargo_or_publish(self):
        fixture = self.fixture()
        fixture.open = False
        before = fixture.head()
        self.succeeds(fixture.run())
        self.assertEqual(fixture.head(), before)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_waiting_cohort_does_not_run_cargo_or_publish(self):
        fixture = self.fixture(waiting=True)
        before = fixture.head()
        self.succeeds(fixture.run())
        self.assertEqual(fixture.head(), before)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_moved_cohort_preserves_main_and_branch_work(self):
        fixture = self.fixture()
        self.succeeds(fixture.run())
        self.assertIn(f"RELAY_SHA={NEW}", fixture.read("ci/pins.env"))
        self.assertIn(f"RELAY_SHA={OLD}", fixture.read("ci/pins.env", "main"))
        self.assertEqual(fixture.read("unrelated.rs"), "preserve branch Rust fixes")
        self.assertEqual(fixture.read("ci/update-pins.sh"), fixture.read("ci/update-pins.sh", "main"))
        self.assertNotIn("start_branch", fixture.posts[0])
        self.assertNotIn("start_sha", fixture.posts[0])
        self.assertEqual({a["action"] for a in fixture.posts[0]["actions"]}, {"update"})

    def test_fork_metadata_is_not_executed_or_published(self):
        fixture = self.fixture()
        fixture.source_project_id = 99
        before = fixture.head()
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(fixture.head(), before)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_untrusted_metadata_is_rejected_before_cargo(self):
        mutations = {
            "executable pins": ("ci/pins.env", 'CORE_TAG=$(touch forbidden)\n'),
            "duplicate pins": ("ci/pins.env", f"RELAY_SHA={OLD}\nRELAY_SHA={NEW}\n"),
            "manifest build hook": ("Cargo.toml", '[package]\nname="vauchi-e2e-tests"\nversion="0.1.0"\nbuild="payload.rs"\n'),
            "foreign lock source": ("Cargo.lock", 'version=4\n[[package]]\nname="evil"\nversion="1.0.0"\nsource="git+https://attacker.invalid/repo#' + OLD + '"\n'),
            "foreign local package": ("Cargo.lock", 'version=4\n[[package]]\nname="foreign"\nversion="1.0.0"\n'),
        }
        for label, (name, value) in mutations.items():
            with self.subTest(label=label):
                fixture = self.fixture()
                fixture.change({name: value})
                before = fixture.head()
                result = fixture.run()
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertEqual(fixture.head(), before)
                self.assertFalse(fixture.cargo_log.exists())
                self.assertEqual(fixture.posts, [])

    def test_symlink_metadata_is_rejected(self):
        fixture = self.fixture()
        path = fixture.src / "ci/pins.env"
        path.unlink()
        path.symlink_to("../Cargo.toml")
        fixture.git("add", ".")
        fixture.git("commit", "-qm", "symlink input")
        fixture.git("push", "-q", str(fixture.remote), f"HEAD:refs/heads/{BRANCH}")
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_unrelated_concurrent_commit_is_preserved(self):
        fixture = self.fixture()
        fixture.on_post = lambda: fixture.change({"unrelated.rs": "newer source work\n"})
        self.succeeds(fixture.run())
        self.assertEqual(fixture.read("unrelated.rs"), "newer source work")
        self.assertIn(f"RELAY_SHA={NEW}", fixture.read("ci/pins.env"))

    def test_stale_file_rejects_the_entire_update(self):
        fixture = self.fixture()
        original_lock = fixture.read("Cargo.lock")
        fixture.on_post = lambda: fixture.change({"ci/pins.env": fixture.read("ci/pins.env") + "\n# newer pins edit\n"})
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("newer pins edit", fixture.read("ci/pins.env"))
        self.assertEqual(fixture.read("Cargo.lock"), original_lock)

    def test_deleted_branch_is_not_recreated(self):
        fixture = self.fixture()
        fixture.on_post = lambda: fixture.git("--git-dir", str(fixture.remote), "update-ref", "-d", f"refs/heads/{BRANCH}")
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(fixture.git("--git-dir", str(fixture.remote), "branch", "--list", BRANCH), "")

    def test_lost_post_response_is_reconciled_without_duplicate_write(self):
        fixture = self.fixture()
        fixture.drop_reply = True
        self.succeeds(fixture.run())
        self.assertEqual(len(fixture.posts), 1)
        self.assertIn(f"RELAY_SHA={NEW}", fixture.read("ci/pins.env"))


if __name__ == "__main__":
    unittest.main()

