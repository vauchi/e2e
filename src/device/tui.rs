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

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tempfile::TempDir;
use tokio::sync::Mutex;

use super::tui_pty::{PtySession, strip_ansi};
use super::{Contact, ContactCard, Device, DeviceType, NetworkConfig};
use crate::error::{E2eError, E2eResult};

/// Find the TUI binary in the workspace.
fn find_tui_binary() -> E2eResult<PathBuf> {
    // CI builds the TUI in a job-private directory and names the binary
    // here; the sibling-checkout guesses below are the developer layout.
    if let Ok(explicit) = std::env::var("VAUCHI_TUI_BIN") {
        let path = PathBuf::from(&explicit);
        if path.is_file() {
            return Ok(path);
        }
        return Err(E2eError::device(format!(
            "VAUCHI_TUI_BIN is set to '{explicit}' but no file exists there"
        )));
    }

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

/// Thread-safe wrapper for PTY session.
struct TuiSession {
    pty: Mutex<Option<PtySession>>,
    is_running: Mutex<bool>,
    tui_path: PathBuf,
    data_dir_path: PathBuf,
    relay_url: String,
    extra_env: std::collections::HashMap<String, String>,
}

impl TuiSession {
    fn new(
        tui_path: PathBuf,
        data_dir_path: PathBuf,
        relay_url: String,
        extra_env: std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            pty: Mutex::new(None),
            is_running: Mutex::new(false),
            tui_path,
            data_dir_path,
            relay_url,
            extra_env,
        }
    }

    async fn ensure_started(&self) -> E2eResult<()> {
        let mut is_running = self.is_running.lock().await;
        if *is_running {
            return Ok(());
        }

        let mut pty_guard = self.pty.lock().await;

        // Create PTY session with wrapper script for proper terminal setup
        let session = PtySession::new(
            &self.tui_path,
            &self.data_dir_path,
            &self.relay_url,
            &self.extra_env,
        )?;
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

    async fn send_alt(&self, c: char) -> E2eResult<()> {
        // crossterm decodes Alt+<char> as ESC followed by the character in
        // the same read; a pause between them is a bare Esc and a typed
        // letter instead.
        self.send_text(&format!("\x1b{c}")).await
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

    /// Poll the terminal until its *rendered* text (ANSI stripped, drawn
    /// over a rolling buffer of recent frames) contains any of the `|`-
    /// separated needles. Robust where `expect_timeout` is not: the TUI
    /// redraws the whole screen each frame, so cursor-position escapes
    /// split a title across the raw stream that `expectrl` matches.
    async fn wait_for_visible(&self, patterns: &str, timeout: Duration) -> E2eResult<String> {
        let needles: Vec<&str> = patterns.split('|').collect();
        let deadline = std::time::Instant::now() + timeout;
        let mut buffer = String::new();
        loop {
            buffer.push_str(&self.read_screen().await?);
            if buffer.len() > 65536 {
                buffer = buffer.split_off(buffer.len() - 65536);
            }
            let visible = strip_ansi(&buffer);
            if let Some(found) = needles.iter().find(|n| visible.contains(**n)) {
                return Ok((*found).to_string());
            }
            if std::time::Instant::now() >= deadline {
                let visible = strip_ansi(&buffer);
                let tail: String = visible
                    .chars()
                    .rev()
                    .take(600)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                return Err(E2eError::device(format!(
                    "none of [{patterns}] became visible within {timeout:?}; tail: {tail}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Repaint and return the whole frame — see `PtySession::redraw_screen`.
    async fn redraw_screen(&self) -> E2eResult<String> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.redraw_screen()
    }

    async fn read_screen(&self) -> E2eResult<String> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.read_available()
    }

    /// Activate the primary contextual action (e.g. "Create new identity").
    async fn activate_primary(&self) -> E2eResult<()> {
        self.send_enter().await
    }

    /// Open the navigation overlay and select the item whose label contains
    /// `label` by its 1-based position.
    async fn navigate_to(&self, label: &str) -> E2eResult<()> {
        self.send_alt('m').await?;
        // The overlay draws the number and the label as separate cell runs,
        // so a cursor-move escape (never a newline) sits between "2." and
        // "Contacts". Expecting the pair also skips any main-screen output
        // still buffered from before the overlay opened.
        let pattern = format!(r"(?i)(\d+)\.(?:\x1b\[[0-9;]*[A-Za-z]|\s)*{label}");
        let matched = self
            .expect_timeout(&pattern, Duration::from_secs(10))
            .await
            .map_err(|_| {
                E2eError::device(format!("Navigation item '{}' not found in overlay", label))
            })?;
        let digits: String = matched.chars().take_while(|c| c.is_ascii_digit()).collect();
        let index: usize = digits.parse().map_err(|_| {
            E2eError::device(format!(
                "Navigation index for '{}' unreadable: {}",
                label, matched
            ))
        })?;
        let digit = char::from_digit(index as u32, 10).ok_or_else(|| {
            E2eError::device(format!("Navigation index {} out of digit range", index))
        })?;
        self.send_char(digit).await
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
    session: TuiSession,
}

impl TuiDevice {
    /// Create a new TUI device with an isolated data directory.
    pub fn new(
        name: impl Into<String>,
        relay_url: impl Into<String>,
        extra_env: std::collections::HashMap<String, String>,
    ) -> E2eResult<Self> {
        let data_dir = TempDir::new()
            .map_err(|e| E2eError::device(format!("Failed to create temp directory: {}", e)))?;

        let tui_path = find_tui_binary()?;
        let relay_url = relay_url.into();
        let data_dir_path = data_dir.path().to_path_buf();

        let session = TuiSession::new(tui_path, data_dir_path, relay_url.clone(), extra_env);

        Ok(Self {
            name: name.into(),
            data_dir,
            relay_url,
            session,
        })
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

    async fn create_identity(&self, name: &str) -> E2eResult<()> {
        self.session.ensure_started().await?;

        // Wait for the generic onboarding screen rendered by Core.
        self.session
            .expect_timeout(
                "Welcome to Vauchi|Create new identity",
                Duration::from_secs(15),
            )
            .await?;

        // Primary action is "Create new identity".
        self.session.activate_primary().await?;

        // default_name screen: type the display name.
        self.session
            .expect_timeout("What's your name?|Display name", Duration::from_secs(10))
            .await?;
        self.session.send_text(name).await?;
        // Return inside a field reports a submission, which Core's name step
        // ignores; Tab leaves the field so Return reaches Continue.
        self.session.send_tab().await?;
        self.session.activate_primary().await?;

        // groups_setup: continue without selecting groups.
        self.session
            .expect_timeout(
                "Choose your groups|Suggested groups",
                Duration::from_secs(10),
            )
            .await?;
        self.session.activate_primary().await?;

        // contact_info: continue without adding fields.
        self.session
            .expect_timeout("Add contact|contact info", Duration::from_secs(10))
            .await?;
        self.session.activate_primary().await?;

        // what_next: start using the app.
        self.session
            .expect_timeout("What would you like|Start using", Duration::from_secs(10))
            .await?;
        self.session.activate_primary().await?;

        // Wait for the main screen to settle.
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(())
    }

    async fn has_identity(&self) -> bool {
        // The TUI persists identity data in a SQLite database.
        let identity_path = self.data_dir.path().join("vauchi.db");
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

    /// Link mode: the camera-less TUI leads its picker with Link, whose
    /// share screen prints the `vauchi://exchange?…` URL as text. The
    /// harness treats that URL as the "QR" payload a peer completes with.
    async fn generate_qr(&self) -> E2eResult<String> {
        self.session.ensure_started().await?;
        self.session.navigate_to("Exchange").await?;
        self.session
            .expect_timeout("Exchange Mode|Pick a way", Duration::from_secs(10))
            .await?;
        self.session.send_char('1').await?;
        self.session
            .expect_timeout("Share Link|Send this link", Duration::from_secs(10))
            .await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let screen = self.session.read_screen().await?;
        let plain = strip_ansi(&screen);
        for token in plain.split_whitespace() {
            if let Some(start) = token.find("vauchi://exchange?") {
                let link = token[start..].trim_end_matches(|c: char| {
                    !c.is_ascii_alphanumeric() && c != '=' && c != '_' && c != '-'
                });
                if link.contains("pk=") {
                    return Ok(link.to_string());
                }
            }
        }
        Err(E2eError::device(
            "Could not extract the share link from the TUI",
        ))
    }

    /// Paste the peer's link on the Link share screen and accept the
    /// exchange request core raises for it (camera-less route,
    /// `problems/2026-09-09-tui-cannot-ingest-peer-exchange-payload`).
    async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()> {
        self.session.ensure_started().await?;
        self.session.navigate_to("Exchange").await?;
        self.session
            .expect_timeout("Exchange Mode|Pick a way", Duration::from_secs(10))
            .await?;
        self.session.send_char('1').await?;
        self.session
            .expect_timeout("Share Link|Send this link", Duration::from_secs(10))
            .await?;
        // Typing lands in the first input (the peer-link field), and Return
        // submits it — core routes the pasted link like an opened deep link
        // and raises the consent screen.
        self.session.send_text(qr_data).await?;
        // Let every pasted character be read and applied before Return:
        // sending Return too soon makes the app process it while the input
        // is still filling, and it activates the primary "Share" action
        // instead of submitting the link.
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.session.send_enter().await?;
        self.session
            .wait_for_visible("Exchange Request|Accept Exchange", Duration::from_secs(20))
            .await?;
        // Return with no focused input activates the context-bar primary
        // ("Accept Exchange"), which retrieves the peer's card and completes.
        self.session.send_enter().await?;
        self.session
            .wait_for_visible(
                "Contact added|Contact Added|Exchange complete|Exchange Complete|Contacts",
                Duration::from_secs(60),
            )
            .await?;
        Ok(())
    }

    async fn await_exchange_complete(&self) -> E2eResult<()> {
        // The initiator never leaves its Link share screen; its live session
        // keeps polling the relay, so once the responder deposits its epk and
        // card the initiator retrieves them and Core raises the completion
        // screen. Read it here — re-navigating would tear down the very
        // session the responder is converging with.
        // `wait_for_visible` (not `expect_timeout`): it polls by draining the
        // PTY on a 200 ms async tick, so running this concurrently with the
        // responder's wait keeps *both* terminals drained. A blocking expect
        // would let the unread terminal fill its PTS buffer and stall its
        // poll loop — the initiator would then never deposit its card.
        self.session
            .wait_for_visible(
                "Contact added|Contact Added|Exchange complete|Exchange Complete|Contacts",
                Duration::from_secs(60),
            )
            .await?;
        Ok(())
    }

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
                if line.contains("Link code:")
                    && let Some(code_start) = line.find(':')
                {
                    return Ok(line[code_start + 1..].trim().to_string());
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

    async fn sync(&self) -> E2eResult<()> {
        self.session.ensure_started().await?;

        // The sync chrome chip sits on every top-level screen; Alt+S is the
        // terminal's key for it (other shells tap it). Core runs the relay
        // catch-up and re-renders with the chip's outcome label.
        self.session.navigate_to("Contacts").await?;
        self.session
            .wait_for_visible("Add Contact|Contacts", Duration::from_secs(10))
            .await?;
        self.session.send_alt('s').await?;
        // Core runs the catch-up synchronously inside the key handler and
        // re-renders the chip: "Sync" becomes "Synced", "Sync failed", or
        // "Sync in N s" when core's own throttle declines a repeat within a
        // minute (core !1591) — that last one is core's decision, not a
        // harness failure. ratatui redraws only the changed cells, so what
        // reaches the PTY is the appended "ed", " failed" or " in N s" —
        // never the whole label.
        let outcome = self
            .session
            .wait_for_visible("failed| in |ed", Duration::from_secs(10))
            .await?;
        if outcome == "failed" {
            return Err(E2eError::device(
                "TUI sync failed (sync chip reports failure)",
            ));
        }

        Ok(())
    }

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        self.session.ensure_started().await?;

        self.session.navigate_to("Contacts").await?;
        self.session
            .wait_for_visible("Add Contact|Contacts", Duration::from_secs(10))
            .await?;
        // The list is already painted by now, so a plain read sees nothing:
        // repaint the frame and parse the list rows ("• Name" / "> Name").
        let frame = strip_ansi(&self.session.redraw_screen().await?);

        // Cursor moves become spaces in `strip_ansi`, so rows are not lines;
        // the box border ("│") is what bounds a cell. Two repaints may land
        // in one read, hence the dedup.
        let mut seen = std::collections::HashSet::new();
        let contacts = frame
            .split('│')
            .map(str::trim)
            .filter_map(|cell| cell.strip_prefix("• ").or_else(|| cell.strip_prefix("> ")))
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty() && seen.insert(name.clone()))
            .map(|name| Contact {
                name,
                id: None,
                verified: false,
            })
            .collect();

        Ok(contacts)
    }

    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>> {
        let contacts = self.list_contacts().await?;
        Ok(contacts.into_iter().find(|c| c.name.contains(name_or_id)))
    }

    async fn get_card(&self) -> E2eResult<ContactCard> {
        self.session.ensure_started().await?;

        tokio::time::sleep(Duration::from_millis(300)).await;
        let screen = self.session.read_screen().await?;

        let mut name = String::new();
        let mut fields = Vec::new();

        for line in screen.lines() {
            let trimmed = line.trim();
            if (trimmed.contains("📇") || trimmed.contains("Card:"))
                && let Some(card_name) = trimmed.split(':').nth(1)
            {
                name = card_name.trim().to_string();
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

    async fn set_network(&self, _config: NetworkConfig) -> E2eResult<()> {
        Ok(())
    }

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

// INLINE_TEST_REQUIRED: tests access private TuiDevice terminal parsing and screen detection

// INLINE_TEST_REQUIRED: constructs TuiDevice through the private binary
// locator, which the integration tests never reach directly.
#[cfg(test)]
mod tests {
    use super::*;

    // @scenario: tui_harness :: CI names the TUI binary explicitly
    /// On a shared shell runner `../tui` is the tui project's own job
    /// workspace, so the sibling-checkout guess must yield to an explicit
    /// path (allyson, 2026-09-12: two jobs wiped each other's build tree).
    #[test]
    fn an_explicit_tui_binary_path_wins_over_the_sibling_guess() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("vauchi-tui");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        // SAFETY: test-local env, single-threaded access to this variable.
        unsafe { std::env::set_var("VAUCHI_TUI_BIN", &bin) };
        let found = find_tui_binary();
        unsafe { std::env::remove_var("VAUCHI_TUI_BIN") };
        assert_eq!(found.unwrap(), bin);
    }

    // @internal
    #[test]
    fn a_dangling_explicit_tui_binary_path_is_an_error_not_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        unsafe { std::env::set_var("VAUCHI_TUI_BIN", &missing) };
        let found = find_tui_binary();
        unsafe { std::env::remove_var("VAUCHI_TUI_BIN") };
        assert!(
            found.is_err(),
            "a wrong explicit path must not fall back to a sibling guess"
        );
    }

    #[test]
    fn test_tui_device_type() {
        // This test will fail if binary doesn't exist, which is expected
        if let Ok(device) = TuiDevice::new(
            "test",
            "ws://localhost:8080",
            std::collections::HashMap::new(),
        ) {
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
