// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Raw PTY plumbing under the TUI device driver: spawning the TUI
//! process inside a PTY, reading and matching its stream,
//! and retaining rendered terminal state across incremental redraws.

use std::time::Duration;

use regex::bytes::Regex;

use crate::error::{E2eError, E2eResult};

mod process;
use process::TerminalProcess;

/// Default timeout for expect operations.
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct PtySession {
    process: TerminalProcess,
    screen: vt100::Parser,
    pending: Vec<u8>,
    closed: bool,
}

impl PtySession {
    pub(super) fn new(
        tui_binary: &std::path::Path,
        data_dir: &std::path::Path,
        relay_url: &str,
        extra_env: &std::collections::HashMap<String, String>,
    ) -> E2eResult<Self> {
        let process = TerminalProcess::new(tui_binary, data_dir, relay_url, extra_env)
            .map_err(|e| E2eError::device(format!("Failed to spawn TUI: {}", e)))?;

        Ok(Self {
            process,
            screen: vt100::Parser::new(40, 160, 0),
            pending: Vec::new(),
            closed: false,
        })
    }

    pub(super) fn send_key(&mut self, key: u8) -> E2eResult<()> {
        self.process
            .write_all(&[key])
            .map_err(|e| E2eError::device(format!("Failed to send key: {}", e)))?;
        Ok(())
    }

    pub(super) fn send_text(&mut self, text: &str) -> E2eResult<()> {
        self.process
            .write_all(text.as_bytes())
            .map_err(|e| E2eError::device(format!("Failed to send text: {}", e)))?;
        Ok(())
    }

    pub(super) fn expect(&mut self, pattern: &str) -> E2eResult<String> {
        self.expect_with_timeout(pattern, DEFAULT_TIMEOUT)
    }

    pub(super) fn expect_with_timeout(
        &mut self,
        pattern: &str,
        timeout: Duration,
    ) -> E2eResult<String> {
        let regex = Regex::new(pattern)
            .map_err(|error| E2eError::device(format!("Invalid terminal pattern: {error}")))?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(found) = regex.find(&self.pending) {
                let end = found.end();
                let matched = String::from_utf8_lossy(found.as_bytes()).into_owned();
                self.screen.process(&self.pending[..end]);
                self.pending.drain(..end);
                return Ok(matched);
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(E2eError::device(format!(
                    "Pattern '{pattern}' not found: timeout"
                )));
            }
            match self.receive(remaining) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(E2eError::device(format!(
                        "Pattern '{pattern}' not found: timeout"
                    )));
                }
                Err(error) => {
                    return Err(E2eError::device(format!(
                        "Pattern '{pattern}' not found: {error}"
                    )));
                }
            }
        }
    }

    /// Read the visible screen without relying on a new full redraw.
    pub(super) fn read_screen_snapshot(&mut self) -> E2eResult<String> {
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        while std::time::Instant::now() < deadline {
            if self.read_available()?.is_empty() {
                break;
            }
        }
        Ok(self.screen.screen().contents())
    }

    pub(super) fn read_available(&mut self) -> E2eResult<String> {
        if self.closed {
            return Err(E2eError::device("TUI output closed unexpectedly"));
        }
        if self.pending.is_empty() {
            self.receive(Duration::ZERO)?;
        }
        self.screen.process(&self.pending);
        let content = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        Ok(content)
    }

    fn receive(&mut self, timeout: Duration) -> E2eResult<bool> {
        if self.closed {
            return Err(E2eError::device("TUI output closed unexpectedly"));
        }
        let chunk = self
            .process
            .read(timeout)
            .map_err(|error| E2eError::device(format!("Failed to read TUI output: {error}")))?;
        let Some(chunk) = chunk else {
            return Ok(false);
        };
        if chunk.is_empty() {
            self.closed = true;
            return Err(E2eError::device("TUI output closed unexpectedly"));
        }
        if self.pending.len() + chunk.len() > 1024 * 1024 {
            return Err(E2eError::device("TUI unmatched output exceeded 1 MiB"));
        }
        self.pending.extend(chunk);
        Ok(true)
    }

    pub(super) fn quit(&mut self) -> E2eResult<()> {
        let _ = self.process.write_all(b"q");
        Ok(())
    }
}

/// Drop terminal escape sequences so text matching sees what the user sees.
pub(super) fn strip_ansi(screen: &str) -> String {
    let mut out = String::with_capacity(screen.len());
    let mut chars = screen.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                let mut terminator = '\0';
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        terminator = d;
                        break;
                    }
                }
                // Cursor-move escapes ('H'/'f' absolute, 'A'-'D' relative)
                // separate two on-screen rows or cells. Dropping them
                // entirely fuses adjacent text (e.g. a URL and the next
                // label), which then reads as one word. Emit a space so
                // word-boundary matching still works.
                if matches!(terminator, 'H' | 'f' | 'A' | 'B' | 'C' | 'D') {
                    out.push(' ');
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

// INLINE_TEST_REQUIRED: PtySession is a private transport helper; real-PTY
// regressions exercise it without exposing it through the public device API.
#[cfg(test)]
mod tests;
