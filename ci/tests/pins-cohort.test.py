# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

# @internal — real Git and HTTP boundaries, synthetic credentials only.
from pathlib import Path
import importlib.util
import py_compile
import shutil
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

    def test_cached_python_bytecode_cannot_run_with_the_write_credential(self):
        fixture = self.fixture()
        entry = fixture.root / "entry"
        entry.mkdir()
        for name in ("pins-cohort.sh", "pins-cohort.py", "pins_cohort_data.py"):
            shutil.copy(ENTRY.parent / name, entry / name)
        payload = fixture.root / "cached-payload.py"
        payload.write_text("import os\nfrom pathlib import Path\n"
                           + f"Path({str(fixture.sentinel)!r}).write_text('write credential available' if os.environ.get('PROJECT_ACCESS_TOKEN') else 'no credential')\n"
                           + "raise SystemExit(99)\n")
        cache = Path(importlib.util.cache_from_source(str(entry / "pins_cohort_data.py")))
        cache.parent.mkdir()
        py_compile.compile(str(payload), cfile=str(cache),
                           invalidation_mode=py_compile.PycInvalidationMode.UNCHECKED_HASH)
        fixture.entry = entry / "pins-cohort.sh"
        result = fixture.run()
        self.assertFalse(fixture.sentinel.exists(), "untrusted cached code ran in the credential holder")
        self.succeeds(result)
        self.assertIn(f"RELAY_SHA={NEW}", fixture.read("ci/pins.env"))

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

    def test_non_green_component_does_not_advance_or_run_cargo(self):
        fixture = self.fixture()
        fixture.pipeline_status = "failed"
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
        self.assertTrue(fixture.pipeline_authentication)
        self.assertEqual(set(fixture.pipeline_authentication), {True})

    def bump_core(self, fixture):
        fixture.change({name: fixture.read(name).replace("0.106.1", "0.106.2") + "\n"
                        for name in ("ci/pins.env", "Cargo.toml")})

    def test_incomplete_core_bump_waits_for_compatible_components(self):
        fixture = self.fixture()
        self.bump_core(fixture)
        before = fixture.head()
        self.succeeds(fixture.run())
        self.assertEqual(fixture.head(), before)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_cli_ready_alone_waits_for_the_relay_core_bump(self):
        fixture = self.fixture(cli_only=True)
        self.bump_core(fixture)
        fixture.component_tags["cli"] = "v0.106.2"
        before = fixture.head()
        self.succeeds(fixture.run())
        self.assertEqual(fixture.head(), before)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_compatible_component_mains_complete_the_core_bump(self):
        fixture = self.fixture()
        self.bump_core(fixture)
        fixture.heads["cli"] = NEW
        fixture.component_tags.update(relay="v0.106.2", cli="v0.106.2")
        self.succeeds(fixture.run())
        self.assertIn("CORE_TAG=v0.106.2", fixture.read("ci/pins.env"))
        self.assertIn("tag=v0.106.2", fixture.read("Cargo.lock"))
        self.assertNotIn("tag=v0.106.1", fixture.read("Cargo.lock"))

    def test_unprotected_core_tag_is_not_release_authority(self):
        fixture = self.fixture()
        fixture.core_protected = False
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_core_tag_cannot_move_during_resolution(self):
        fixture = self.fixture()
        fixture.core_tag_commits = ["c" * 40, "d" * 40]
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("Core release tag moved", result.stderr)
        self.assertEqual(fixture.posts, [])

    def test_component_api_authorization_failure_is_not_reported_as_waiting(self):
        fixture = self.fixture()
        fixture.pipeline_http_status = 403
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("HTTP 403", result.stderr)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_missing_or_malformed_file_revision_blocks_publication(self):
        for value in (None, "not-a-commit"):
            with self.subTest(revision=value):
                fixture = self.fixture()
                fixture.file_metadata_override = {"last_commit_id": value}
                result = fixture.run()
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertFalse(fixture.cargo_log.exists())
                self.assertEqual(fixture.posts, [])

    def test_unprotected_component_main_is_not_admitted(self):
        fixture = self.fixture()
        fixture.component_main_protected = False
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_closed_mr_is_not_updated(self):
        fixture = self.fixture()
        fixture.on_recheck = lambda: setattr(fixture, "open", False)
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(fixture.posts, [])

    def test_metadata_formatting_is_preserved(self):
        fixture = self.fixture()
        fixture.change({"ci/pins.env": fixture.read("ci/pins.env") + "\n\n# retained branch comment\n\n"})
        self.succeeds(fixture.run())
        self.assertTrue(fixture.read_bytes("ci/pins.env").endswith(b"\n\n# retained branch comment\n\n"))

    def test_path_core_dependency_is_not_a_matching_release(self):
        fixture = self.fixture()
        fixture.component_extra["relay"] = '\nvauchi-app={path="branch-local-engine"}\n'
        before = fixture.head()
        self.succeeds(fixture.run())
        self.assertEqual(fixture.head(), before)
        self.assertFalse(fixture.cargo_log.exists())
        self.assertEqual(fixture.posts, [])

    def test_successful_resolver_with_missing_core_cannot_publish(self):
        fixture = self.fixture()
        cargo = fixture.root / "bin/cargo"
        cargo.write_text("#!/bin/sh\nprintf 'version=4\\n[[package]]\\nname=\"vauchi-e2e-tests\"\\nversion=\"0.1.0\"\\n' > Cargo.lock\n")
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(fixture.posts, [])

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
            "foreign dependency identity": ("Cargo.lock", 'version=4\n[[package]]\nname="vauchi-e2e-tests"\nversion="0.1.0"\ndependencies=["evil 1.0.0 (git+https://attacker.invalid/repo#' + OLD + ')"]\n'),
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

    def test_retargeted_mr_is_rejected_before_publication(self):
        fixture = self.fixture()
        fixture.on_recheck = lambda: setattr(fixture, "target_branch", "release")
        before = fixture.head()
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(fixture.head(), before)
        self.assertEqual(fixture.posts, [])

    def test_cli_only_move_guards_the_unchanged_manifest(self):
        fixture = self.fixture(cli_only=True)
        fixture.on_post = lambda: fixture.change({"Cargo.toml": fixture.read("Cargo.toml") + "\n# concurrent manifest edit\n"})
        result = fixture.run()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("concurrent manifest edit", fixture.read("Cargo.toml"))
        self.assertIn(f"CLI_SHA={OLD}", fixture.read("ci/pins.env"))

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
