// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Finding the `vauchi` CLI binary a test drives.

use std::path::PathBuf;

use tempfile::TempDir;

use super::{CliDevice, TEST_CLOCK_ENV};
use crate::error::{E2eError, E2eResult};

impl CliDevice {
    /// CLI paths to try, most-trusted first.
    ///
    /// `E2E_BIN_DIR` is how CI hands over its binaries. `target/e2e-bin`
    /// outranks every plain build because only `just e2e-build` passes
    /// `--features e2e-test-clock`; a CLI without it ignores
    /// `VAUCHI_TEST_CLOCK_EPOCH`, which silently turns clock-driven tests into
    /// same-second tie-breaks (#395). The cli/ crate is its own Cargo
    /// workspace, so its debug build lands in `cli/target/debug`; the
    /// workspace-root paths are historical fallbacks.
    pub fn cli_binary_candidates(env_bin_dir: Option<&str>) -> Vec<PathBuf> {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut candidates = Vec::new();
        if let Some(dir) = env_bin_dir {
            candidates.push(PathBuf::from(dir).join("vauchi"));
        }
        candidates.extend([
            workspace.join("target/e2e-bin/vauchi"),
            workspace.join("cli/target/debug/vauchi"),
            workspace.join("target/debug/vauchi"),
            workspace.join("target/release/vauchi"),
        ]);
        candidates
    }

    /// Whether `cli` is an `e2e-test-clock` build. Such a build panics on a
    /// malformed `VAUCHI_TEST_CLOCK_EPOCH` the moment a command stamps time;
    /// any other build accepts it and keeps the real clock.
    pub fn cli_honors_test_clock(cli: &std::path::Path) -> bool {
        let Ok(scratch) = TempDir::new() else {
            return false;
        };
        let data_dir = scratch.path().to_string_lossy().to_string();
        let run = |args: &[&str], epoch: Option<&str>| {
            let mut command = std::process::Command::new(cli);
            command.args(["--data-dir", &data_dir]).args(args);
            command.env_remove(TEST_CLOCK_ENV);
            if let Some(epoch) = epoch {
                command.env(TEST_CLOCK_ENV, epoch);
            }
            command.output()
        };
        let initialised = run(&["init", "clock-probe"], None).is_ok_and(|out| out.status.success());
        initialised
            && run(
                &["card", "add", "phone", "probe", "+12025550000"],
                Some("not-a-number"),
            )
            .is_ok_and(|out| {
                !out.status.success()
                    && String::from_utf8_lossy(&out.stderr).contains(TEST_CLOCK_ENV)
            })
    }

    /// Find the CLI binary in the workspace.
    pub(super) fn find_cli_binary() -> E2eResult<PathBuf> {
        let env_bin_dir = std::env::var("E2E_BIN_DIR").ok();
        Self::cli_binary_candidates(env_bin_dir.as_deref())
            .into_iter()
            .find(|path| path.exists())
            .ok_or_else(|| {
                E2eError::cli_execution(
                    "CLI binary not found. Run `just e2e-build` — it builds the CLI \
                     with --features e2e-test-clock, which the clock-driven tests \
                     require and a plain `just build cli` omits. To use binaries \
                     from elsewhere, set E2E_BIN_DIR.",
                )
            })
    }
}
