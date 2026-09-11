// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Raw PTY plumbing under the TUI device driver: spawning the wrapper
//! script inside an `expectrl` session, reading and matching its stream,
//! and forcing a repaint when a read must see the whole frame.

use std::io::{Read, Write as IoWrite};
use std::time::Duration;

use expectrl::{Regex, Session};

use crate::error::{E2eError, E2eResult};

/// Default timeout for expect operations.
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Simple wrapper around expectrl Session for PTY control.
/// This is stored separately to avoid complex generics.
pub(super) struct PtySession {
    session: Session,
}

impl PtySession {
    pub(super) fn new(
        tui_binary: &std::path::Path,
        data_dir: &std::path::Path,
        relay_url: &str,
        extra_env: &std::collections::HashMap<String, String>,
    ) -> E2eResult<Self> {
        // Create a wrapper script to handle all the terminal setup
        // This avoids complex shell quoting issues
        let script_path = data_dir.join("run_tui.sh");
        // Forward caller-supplied env (e.g. the e2e OHTTP key/route overrides)
        // into the TUI process so it can reach a locally-spawned relay — the
        // CLI gets these via its subprocess env; the TUI needs them too.
        let extra_exports: String = extra_env
            .iter()
            .map(|(k, v)| format!("export {k}=\"{v}\"\n"))
            .collect();
        let script_content = format!(
            r#"#!/bin/bash
export TERM=xterm-256color
export VAUCHI_DATA_DIR="{}"
export VAUCHI_RELAY_URL="{}"
{}stty rows 40 cols 160 2>/dev/null || true
exec "{}"
"#,
            data_dir.display(),
            relay_url,
            extra_exports,
            tui_binary.display()
        );

        std::fs::write(&script_path, &script_content)
            .map_err(|e| E2eError::device(format!("Failed to write wrapper script: {}", e)))?;

        // Make the script executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path)
                .map_err(|e| E2eError::device(format!("Failed to get script metadata: {}", e)))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).map_err(|e| {
                E2eError::device(format!("Failed to set script permissions: {}", e))
            })?;
        }

        // Spawn the wrapper script directly inside an expectrl PTY. This is
        // portable across Linux and macOS; the older `script -c` invocation
        // works on Linux but is rejected by macOS `script`.
        let script_cmd = script_path.to_string_lossy().to_string();
        let mut session = expectrl::spawn(&script_cmd)
            .map_err(|e| E2eError::device(format!("Failed to spawn TUI: {}", e)))?;

        // Set default timeout
        session.set_expect_timeout(Some(DEFAULT_TIMEOUT));

        Ok(Self { session })
    }

    pub(super) fn send_key(&mut self, key: u8) -> E2eResult<()> {
        self.session
            .write_all(&[key])
            .map_err(|e| E2eError::device(format!("Failed to send key: {}", e)))?;
        Ok(())
    }

    pub(super) fn send_text(&mut self, text: &str) -> E2eResult<()> {
        self.session
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
        self.session.set_expect_timeout(Some(timeout));

        // expectrl's Regex takes the pattern directly
        let regex = Regex(pattern);

        let found = self
            .session
            .expect(regex)
            .map_err(|e| E2eError::device(format!("Pattern '{}' not found: {}", pattern, e)))?;

        let matched = found.get(0).unwrap_or_else(|| found.as_bytes());
        Ok(String::from_utf8_lossy(matched).to_string())
    }

    /// Force the TUI to repaint every cell and return the repainted frame.
    ///
    /// ratatui redraws only the cells that changed since the last frame, so
    /// a screen that is already on display never reaches the PTY again — a
    /// read after navigating to it yields nothing. A window-size change
    /// makes the terminal clear and repaint the whole frame, which is the
    /// one way to read what is on screen without a virtual terminal.
    pub(super) fn redraw_screen(&mut self) -> E2eResult<String> {
        let (cols, rows) = self
            .session
            .get_process()
            .get_window_size()
            .map_err(|e| E2eError::device(format!("Failed to read PTY size: {e}")))?;
        for width in [cols.saturating_sub(1).max(20), cols] {
            self.session
                .get_process_mut()
                .set_window_size(width, rows)
                .map_err(|e| E2eError::device(format!("Failed to resize PTY: {e}")))?;
            std::thread::sleep(Duration::from_millis(250));
        }
        // Bounded by time, not by silence: the TUI re-emits a hide-cursor
        // sequence every frame, so the stream never goes quiet.
        let mut frame = String::new();
        let deadline = std::time::Instant::now() + Duration::from_millis(1500);
        while std::time::Instant::now() < deadline {
            frame.push_str(&self.read_available()?);
        }
        Ok(frame)
    }

    pub(super) fn read_available(&mut self) -> E2eResult<String> {
        // Set very short timeout for non-blocking read
        self.session
            .set_expect_timeout(Some(Duration::from_millis(100)));

        let mut buffer = vec![0u8; 4096];
        match self.session.read(&mut buffer) {
            Ok(n) => {
                let content = String::from_utf8_lossy(&buffer[..n]).to_string();
                Ok(content)
            }
            Err(_) => Ok(String::new()),
        }
    }

    pub(super) fn quit(&mut self) -> E2eResult<()> {
        let _ = self.session.write_all(b"q");
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
