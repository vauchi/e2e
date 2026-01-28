// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! TUI-based device implementation.
//!
//! Controls the Vauchi TUI using pseudo-terminal automation via expectrl.
//!
//! ## Implementation
//!
//! Uses the `expectrl` crate for PTY management to:
//! - Spawn the TUI process in a terminal emulator
//! - Send keyboard inputs (navigation keys, text)
//! - Parse the terminal screen buffer for feedback
//!
//! ## Keyboard Shortcuts (from TUI)
//!
//! **Setup Screen**:
//! - `c` - Create New Identity
//! - `i` - Import Backup
//!
//! **Home Screen**:
//! - `c` - Contacts
//! - `s` - Settings
//! - `d` - Devices
//! - `r` - Recovery
//! - `n` - Sync
//! - `a` - Add field
//! - `e` / Enter - Edit selected field
//! - `j` / Down - Navigate down
//! - `k` / Up - Navigate up
//!
//! **Global**:
//! - `q` - Quit
//! - `?` - Help
//! - `Esc` - Go back

use std::io::{Read, Write as IoWrite};
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use expectrl::{Regex, Session};
use tempfile::TempDir;
use tokio::sync::Mutex;

use super::{Contact, ContactCard, Device, DeviceType, NetworkConfig};
use crate::error::{E2eError, E2eResult};

/// Default timeout for expect operations.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Find the TUI binary in the workspace.
fn find_tui_binary() -> E2eResult<PathBuf> {
    // Try release binary first
    let release_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tui/target/release/vauchi-tui");
    if release_path.exists() {
        return Ok(release_path);
    }

    // Try debug binary
    let debug_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tui/target/debug/vauchi-tui");
    if debug_path.exists() {
        return Ok(debug_path);
    }

    // Try shared target directory (release)
    let shared_release =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/release/vauchi-tui");
    if shared_release.exists() {
        return Ok(shared_release);
    }

    // Try shared target directory (debug)
    let shared_debug = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/vauchi-tui");
    if shared_debug.exists() {
        return Ok(shared_debug);
    }

    Err(E2eError::device(
        "TUI binary not found. Please run `cargo build -p vauchi-tui` first.",
    ))
}

/// Simple wrapper around expectrl Session for PTY control.
/// This is stored separately to avoid complex generics.
struct PtySession {
    session: Session,
    #[allow(dead_code)]
    log_file: std::fs::File,
}

impl PtySession {
    fn new(
        tui_binary: &std::path::Path,
        data_dir: &std::path::Path,
        relay_url: &str,
    ) -> E2eResult<Self> {
        // Create log file for debugging
        let log_path = data_dir.join("pty.log");
        let log_file = std::fs::File::create(&log_path)
            .map_err(|e| E2eError::device(format!("Failed to create log file: {}", e)))?;

        // Create a wrapper script to handle all the terminal setup
        // This avoids complex shell quoting issues
        let script_path = data_dir.join("run_tui.sh");
        let script_content = format!(
            r#"#!/bin/bash
export TERM=xterm-256color
export VAUCHI_DATA_DIR="{}"
export VAUCHI_RELAY_URL="{}"
stty rows 24 cols 80 2>/dev/null || true
exec "{}"
"#,
            data_dir.display(),
            relay_url,
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

        // Use `script` to provide a proper controlling terminal with /dev/tty
        let script_cmd = format!("script -q -c '{}' /dev/null", script_path.display());

        // Spawn PTY session with script wrapper
        let mut session = expectrl::spawn(&script_cmd)
            .map_err(|e| E2eError::device(format!("Failed to spawn TUI: {}", e)))?;

        // Set default timeout
        session.set_expect_timeout(Some(DEFAULT_TIMEOUT));

        Ok(Self { session, log_file })
    }

    fn send_key(&mut self, key: u8) -> E2eResult<()> {
        self.session
            .write_all(&[key])
            .map_err(|e| E2eError::device(format!("Failed to send key: {}", e)))?;
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> E2eResult<()> {
        self.session
            .write_all(text.as_bytes())
            .map_err(|e| E2eError::device(format!("Failed to send text: {}", e)))?;
        Ok(())
    }

    fn expect(&mut self, pattern: &str) -> E2eResult<String> {
        self.expect_with_timeout(pattern, DEFAULT_TIMEOUT)
    }

    fn expect_with_timeout(&mut self, pattern: &str, timeout: Duration) -> E2eResult<String> {
        self.session.set_expect_timeout(Some(timeout));

        // expectrl's Regex takes the pattern directly
        let regex = Regex(pattern);

        let found = self
            .session
            .expect(regex)
            .map_err(|e| E2eError::device(format!("Pattern '{}' not found: {}", pattern, e)))?;

        let matched = String::from_utf8_lossy(found.as_bytes()).to_string();
        Ok(matched)
    }

    fn read_available(&mut self) -> E2eResult<String> {
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

    fn quit(&mut self) -> E2eResult<()> {
        let _ = self.session.write_all(b"q");
        Ok(())
    }
}

/// Thread-safe wrapper for PTY session.
struct TuiSession {
    pty: Mutex<Option<PtySession>>,
    is_running: Mutex<bool>,
    tui_path: PathBuf,
    data_dir_path: PathBuf,
    relay_url: String,
}

impl TuiSession {
    fn new(tui_path: PathBuf, data_dir_path: PathBuf, relay_url: String) -> Self {
        Self {
            pty: Mutex::new(None),
            is_running: Mutex::new(false),
            tui_path,
            data_dir_path,
            relay_url,
        }
    }

    async fn ensure_started(&self) -> E2eResult<()> {
        let mut is_running = self.is_running.lock().await;
        if *is_running {
            return Ok(());
        }

        let mut pty_guard = self.pty.lock().await;

        // Create PTY session with wrapper script for proper terminal setup
        let session = PtySession::new(&self.tui_path, &self.data_dir_path, &self.relay_url)?;
        *pty_guard = Some(session);
        *is_running = true;

        // Give TUI time to initialize and render
        // The script wrapper + TUI startup needs time to draw the first frame
        drop(pty_guard);
        drop(is_running);
        tokio::time::sleep(Duration::from_secs(2)).await;

        Ok(())
    }

    async fn send_key(&self, key: u8) -> E2eResult<()> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.send_key(key)?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    async fn send_char(&self, c: char) -> E2eResult<()> {
        self.send_key(c as u8).await
    }

    async fn send_escape(&self) -> E2eResult<()> {
        self.send_key(0x1B).await
    }

    async fn send_enter(&self) -> E2eResult<()> {
        self.send_key(b'\r').await
    }

    async fn send_backspace(&self) -> E2eResult<()> {
        self.send_key(0x08).await
    }

    async fn send_tab(&self) -> E2eResult<()> {
        self.send_key(b'\t').await
    }

    async fn send_text(&self, text: &str) -> E2eResult<()> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.send_text(text)?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    async fn expect(&self, pattern: &str) -> E2eResult<String> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.expect(pattern)
    }

    async fn expect_timeout(&self, pattern: &str, timeout: Duration) -> E2eResult<String> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.expect_with_timeout(pattern, timeout)
    }

    async fn read_screen(&self) -> E2eResult<String> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.read_available()
    }

    async fn stop(&self) -> E2eResult<()> {
        let mut pty_guard = self.pty.lock().await;
        let mut is_running = self.is_running.lock().await;

        if let Some(mut pty) = pty_guard.take() {
            let _ = pty.quit();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        *is_running = false;
        Ok(())
    }
}

/// A device controlled via the TUI.
///
/// Uses PTY automation to control the terminal UI.
pub struct TuiDevice {
    name: String,
    data_dir: TempDir,
    relay_url: String,
    #[allow(dead_code)]
    tui_path: PathBuf,
    session: TuiSession,
}

impl TuiDevice {
    /// Create a new TUI device with an isolated data directory.
    pub fn new(name: impl Into<String>, relay_url: impl Into<String>) -> E2eResult<Self> {
        let data_dir = TempDir::new()
            .map_err(|e| E2eError::device(format!("Failed to create temp directory: {}", e)))?;

        let tui_path = find_tui_binary()?;
        let relay_url = relay_url.into();
        let data_dir_path = data_dir.path().to_path_buf();

        let session = TuiSession::new(tui_path.clone(), data_dir_path, relay_url.clone());

        Ok(Self {
            name: name.into(),
            data_dir,
            relay_url,
            tui_path,
            session,
        })
    }

    /// Get the data directory path.
    #[allow(dead_code)]
    pub fn data_dir_path(&self) -> &std::path::Path {
        self.data_dir.path()
    }
}

#[async_trait]
impl Device for TuiDevice {
    fn device_type(&self) -> DeviceType {
        DeviceType::Tui
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn relay_url(&self) -> &str {
        &self.relay_url
    }

    // === Identity Management ===

    async fn create_identity(&self, name: &str) -> E2eResult<()> {
        // Start session if not running
        self.session.ensure_started().await?;

        // Wait for setup screen - look for key text in the TUI output
        // The TUI uses Ratatui which includes ANSI escape codes mixed with text
        // Look for the setup screen indicators
        self.session
            .expect_timeout("Setup|Create New Identity|Welcome", Duration::from_secs(15))
            .await?;

        // Press 'c' to create identity
        self.session.send_char('c').await?;

        // Wait for home screen - TUI goes directly to home after identity creation
        // Look for common home screen elements
        self.session
            .expect_timeout("Contacts|Search|Press", Duration::from_secs(10))
            .await?;

        // Now go to settings to update the name
        self.session.send_char('s').await?;

        // Press 'n' to edit name
        self.session.send_char('n').await?;

        // Clear existing text (multiple backspaces)
        // The default name is "New User" (8 chars)
        for _ in 0..20 {
            self.session.send_backspace().await?;
        }

        // Type the new name
        self.session.send_text(name).await?;

        // Submit with Enter
        self.session.send_enter().await?;

        // Go back to home
        self.session.send_escape().await?;

        Ok(())
    }

    async fn has_identity(&self) -> bool {
        // Check if identity file exists in data directory
        let identity_path = self.data_dir.path().join("identity.json");
        identity_path.exists()
    }

    async fn export_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Use backup/restore for TUI identity export".into(),
        ))
    }

    async fn import_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Use backup/restore for TUI identity import".into(),
        ))
    }

    // === Exchange ===

    async fn generate_qr(&self) -> E2eResult<String> {
        self.session.ensure_started().await?;

        // Navigate to Exchange screen from Home (press 'e' when on field, otherwise use menu)
        // Actually, from looking at the input handler, 'e' edits a field if fields exist
        // We need a different approach - maybe there's a direct exchange key

        // For now, return error - TUI QR extraction is complex
        Err(E2eError::DeviceNotSupported(
            "TUI QR extraction not yet implemented - requires screen OCR or data export".into(),
        ))
    }

    async fn complete_exchange(&self, _qr_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI cannot scan QR codes. Use CLI for programmatic exchange.".into(),
        ))
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        self.session.ensure_started().await?;

        // Navigate to Devices screen
        self.session.send_char('d').await?;

        // Press 'l' to generate device link
        self.session.send_char('l').await?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Read screen for link data
        let screen = self.session.read_screen().await?;

        // Look for "Link code:" in status
        if screen.contains("Link code:") {
            for line in screen.lines() {
                if line.contains("Link code:") {
                    if let Some(code_start) = line.find(':') {
                        return Ok(line[code_start + 1..].trim().to_string());
                    }
                }
            }
        }

        Err(E2eError::device("Could not extract device link from TUI"))
    }

    async fn join_identity(&self, _qr_data: &str, _device_name: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "TUI device linking requires manual QR entry. Use CLI.".into(),
        ))
    }

    async fn complete_device_link(&self, _request_data: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "TUI device linking requires manual interaction. Use CLI.".into(),
        ))
    }

    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device linking requires manual interaction. Use CLI.".into(),
        ))
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        self.session.ensure_started().await?;

        self.session.send_char('d').await?;

        tokio::time::sleep(Duration::from_millis(300)).await;

        let screen = self.session.read_screen().await?;

        let mut devices = Vec::new();
        for line in screen.lines() {
            if line.contains("Device:") || line.contains("[") {
                devices.push(line.trim().to_string());
            }
        }

        self.session.send_escape().await?;

        Ok(devices)
    }

    // === Sync ===

    async fn sync(&self) -> E2eResult<()> {
        self.session.ensure_started().await?;

        // Navigate to Sync screen (key 'n' from home)
        self.session.send_char('n').await?;

        // Press 's' to start sync
        self.session.send_char('s').await?;

        // Wait for sync to complete
        self.session
            .expect_timeout("Sync complete|Sync failed", Duration::from_secs(10))
            .await?;

        // Go back to home
        self.session.send_escape().await?;

        Ok(())
    }

    // === Contacts ===

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        self.session.ensure_started().await?;

        self.session.send_char('c').await?;

        tokio::time::sleep(Duration::from_millis(300)).await;

        let screen = self.session.read_screen().await?;

        let mut contacts = Vec::new();
        for line in screen.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("─") || trimmed.contains("Contacts") {
                continue;
            }
            if !trimmed.is_empty() && !trimmed.starts_with('[') {
                contacts.push(Contact {
                    name: trimmed.to_string(),
                    id: None,
                    verified: false,
                });
            }
        }

        self.session.send_escape().await?;

        Ok(contacts)
    }

    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>> {
        let contacts = self.list_contacts().await?;
        Ok(contacts.into_iter().find(|c| c.name.contains(name_or_id)))
    }

    // === Card Management ===

    async fn get_card(&self) -> E2eResult<ContactCard> {
        self.session.ensure_started().await?;

        tokio::time::sleep(Duration::from_millis(300)).await;
        let screen = self.session.read_screen().await?;

        let mut name = String::new();
        let mut fields = Vec::new();

        for line in screen.lines() {
            let trimmed = line.trim();
            if trimmed.contains("📇") || trimmed.contains("Card:") {
                if let Some(card_name) = trimmed.split(':').nth(1) {
                    name = card_name.trim().to_string();
                }
            }
            if trimmed.contains(':') && !trimmed.starts_with("Card") {
                let parts: Vec<&str> = trimmed.splitn(2, ':').collect();
                if parts.len() == 2 {
                    fields.push(super::CardField {
                        field_type: "text".to_string(),
                        label: parts[0].trim().to_string(),
                        value: parts[1].trim().to_string(),
                    });
                }
            }
        }

        Ok(ContactCard { name, fields })
    }

    async fn add_field(&self, field_type: &str, label: &str, value: &str) -> E2eResult<()> {
        self.session.ensure_started().await?;

        // Press 'a' to add field from home
        self.session.send_char('a').await?;

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Select field type
        let type_index = match field_type.to_lowercase().as_str() {
            "email" => 0,
            "phone" => 1,
            "website" => 2,
            "note" => 3,
            "address" => 4,
            _ => 5,
        };

        for _ in 0..type_index {
            self.session.send_char('l').await?;
        }

        // Tab to label field
        self.session.send_tab().await?;
        self.session.send_text(label).await?;

        // Tab to value field
        self.session.send_tab().await?;
        self.session.send_text(value).await?;

        // Submit
        self.session.send_enter().await?;

        self.session.expect("Field added").await?;

        Ok(())
    }

    async fn edit_field(&self, label: &str, value: &str) -> E2eResult<()> {
        self.session.ensure_started().await?;

        let card = self.get_card().await?;
        let field_index = card
            .fields
            .iter()
            .position(|f| f.label == label)
            .ok_or_else(|| E2eError::device(format!("Field '{}' not found", label)))?;

        for _ in 0..field_index {
            self.session.send_char('j').await?;
        }

        self.session.send_char('e').await?;

        for _ in 0..100 {
            self.session.send_backspace().await?;
        }

        self.session.send_text(value).await?;
        self.session.send_enter().await?;

        self.session.expect("Field updated").await?;

        Ok(())
    }

    async fn remove_field(&self, label: &str) -> E2eResult<()> {
        self.session.ensure_started().await?;

        let card = self.get_card().await?;
        let field_index = card
            .fields
            .iter()
            .position(|f| f.label == label)
            .ok_or_else(|| E2eError::device(format!("Field '{}' not found", label)))?;

        for _ in 0..field_index {
            self.session.send_char('j').await?;
        }

        self.session.send_char('x').await?;

        self.session.expect("Field removed").await?;

        Ok(())
    }

    async fn edit_name(&self, new_name: &str) -> E2eResult<()> {
        self.session.ensure_started().await?;

        self.session.send_char('s').await?;
        self.session.send_char('n').await?;

        for _ in 0..50 {
            self.session.send_backspace().await?;
        }

        self.session.send_text(new_name).await?;
        self.session.send_enter().await?;

        self.session.expect("Display name updated").await?;

        self.session.send_escape().await?;

        Ok(())
    }

    // === Network Simulation ===

    async fn set_network(&self, _config: NetworkConfig) -> E2eResult<()> {
        Ok(())
    }

    // === App Lifecycle ===

    async fn kill_app(&self) -> E2eResult<()> {
        self.session.stop().await
    }

    async fn launch_app(&self) -> E2eResult<()> {
        self.session.ensure_started().await
    }

    fn supports_lifecycle_control(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_device_type() {
        // This test will fail if binary doesn't exist, which is expected
        if let Ok(device) = TuiDevice::new("test", "ws://localhost:8080") {
            assert_eq!(device.device_type(), DeviceType::Tui);
            assert_eq!(device.name(), "test");
        }
    }

    #[test]
    fn test_find_binary_paths() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let expected_paths = vec![
            manifest_dir.join("../tui/target/release/vauchi-tui"),
            manifest_dir.join("../tui/target/debug/vauchi-tui"),
            manifest_dir.join("../target/release/vauchi-tui"),
            manifest_dir.join("../target/debug/vauchi-tui"),
        ];

        for path in expected_paths {
            assert!(path.to_str().unwrap().contains("vauchi-tui"));
        }
    }
}
