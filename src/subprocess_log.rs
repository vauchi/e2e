// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subprocess stdout/stderr drain helper for orchestrator-spawned
//! relays and CLIs.
//!
//! Each spawned subprocess (vauchi-relay, vauchi-ohttp-relay) gets its
//! pipe stream drained on a dedicated **OS thread** rather than a
//! tokio task. Reasons:
//!
//! - The drain must survive runtime teardown. When a `#[tokio::test]`
//!   panics, the runtime is dropped and any outstanding `tokio::spawn`
//!   tasks are aborted mid-read — losing pending lines, including the
//!   ones that explain *why* the test failed.
//! - tokio's `ChildStderr`/`ChildStdout` are async-only. Converting
//!   them to a raw fd and reading them with `std::io::BufReader`
//!   avoids any tokio dependency for the drain itself.
//!
//! See `_private/docs/problems/2026-04-27-e2e-sync-http-400/` for the
//! incident that prompted this.

use std::collections::VecDeque;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};

const MAX_CAPTURED_OUTPUT_LINES: usize = 1_024;

/// Bounded ring buffer of a subprocess's most recent stdout/stderr lines,
/// shared between the drain threads and the test that inspects them.
#[derive(Clone, Default)]
pub struct OutputCapture {
    lines: Arc<Mutex<VecDeque<String>>>,
}

impl OutputCapture {
    pub fn record(&self, line: &str) {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if lines.len() == MAX_CAPTURED_OUTPUT_LINES {
            lines.pop_front();
        }
        lines.push_back(line.to_string());
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

/// Spawn an OS thread that reads `pipe` line-by-line and calls
/// `on_line` for each line. Runs until EOF (i.e. until the subprocess
/// closes the pipe — typically on exit).
///
/// `thread_name` is used both for the OS thread name and as a tag in
/// any `panic`-time diagnostics. `on_line` runs on the spawned thread.
pub fn drain_pipe<F>(pipe: OwnedFd, thread_name: String, on_line: F)
where
    F: Fn(&str) + Send + 'static,
{
    let file = std::fs::File::from(pipe);
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            use std::io::BufRead as _;
            let reader = std::io::BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                on_line(&line);
            }
        })
        .expect("subprocess log drain thread");
}

// INLINE_TEST_REQUIRED: the ring-buffer bound is a private constant.
#[cfg(test)]
mod tests {
    use super::*;

    // @internal
    #[test]
    fn output_capture_is_bounded() {
        let capture = OutputCapture::default();
        for index in 0..=MAX_CAPTURED_OUTPUT_LINES {
            capture.record(&format!("line-{index}"));
        }

        let lines = capture.snapshot();
        assert_eq!(lines.len(), MAX_CAPTURED_OUTPUT_LINES);
        assert_eq!(lines.first(), Some(&"line-1".to_string()));
        assert_eq!(
            lines.last(),
            Some(&format!("line-{MAX_CAPTURED_OUTPUT_LINES}"))
        );
    }
}
