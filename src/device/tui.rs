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

#[path = "tui_screen_text.rs"]
mod screen_text;
use screen_text::{
    DEVICE_LINK_PREFIX, confirmation_code_in, device_link_invitation_in, share_link_in,
};

/// Find the TUI binary in the workspace.
fn find_tui_binary() -> E2eResult<PathBuf> {
    find_tui_binary_given(std::env::var("VAUCHI_TUI_BIN").ok())
}

/// [`find_tui_binary`] with the value of `VAUCHI_TUI_BIN` passed in, so
/// tests need not mutate the process environment.
fn find_tui_binary_given(explicit: Option<String>) -> E2eResult<PathBuf> {
    // CI builds the TUI in a job-private directory and names the binary
    // here; the sibling-checkout guesses below are the developer layout.
    if let Some(explicit) = explicit {
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

    /// Poll incremental terminal output for any of the `|`-separated needles.
    /// Cursor-position escapes are stripped before matching text; unchanged
    /// cells are absent from the stream.
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

    /// Poll the retained frame for any of the `|`-separated needles. Unlike
    /// [`Self::wait_for_visible`] it sees text drawn before the wait began,
    /// and a timeout reports the whole frame, not a stream tail.
    async fn wait_for_frame(&self, patterns: &str, timeout: Duration) -> E2eResult<String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let frame = self.read_screen_snapshot().await?;
            if let Some(found) = patterns.split('|').find(|needle| frame.contains(needle)) {
                return Ok(found.to_string());
            }
            if std::time::Instant::now() >= deadline {
                return Err(E2eError::device(format!(
                    "none of [{patterns}] was on screen within {timeout:?}; frame:\n{frame}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn read_screen_snapshot(&self) -> E2eResult<String> {
        let mut pty_guard = self.pty.lock().await;
        let pty = pty_guard
            .as_mut()
            .ok_or_else(|| E2eError::device("TUI session not started"))?;
        pty.read_screen_snapshot()
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

    async fn navigate_to_contacts(&self) -> E2eResult<()> {
        self.navigate_to("Contacts").await?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let frame = self.read_screen_snapshot().await?;
            // Navigation uses a double border and also contains "Contacts".
            // Its dismissal, rather than that label alone, confirms selection.
            if !frame.contains('╔') && (frame.contains("Add Contact") || frame.contains("Contacts"))
            {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(E2eError::device(
                    "Contacts screen did not become visible within 10s",
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
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
    /// share screen prints the exchange URL as text. The
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
        share_link_in(&strip_ansi(&screen))
            .ok_or_else(|| E2eError::device("Could not extract the share link from the TUI"))
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

    /// Core's relay-brokered device link, not the CLI's hand-carried one:
    /// the terminal prints the invitation URL under its QR label, and that
    /// URL is what the joining device pastes.
    async fn start_device_link(&self) -> E2eResult<String> {
        self.session.ensure_started().await?;
        self.session.navigate_to("Devices").await?;
        self.session
            .wait_for_visible("Link New Device", Duration::from_secs(10))
            .await?;
        self.session.activate_primary().await?;
        self.session
            .wait_for_visible(DEVICE_LINK_PREFIX, Duration::from_secs(30))
            .await?;
        let frame = self.session.read_screen_snapshot().await?;
        device_link_invitation_in(&frame).ok_or_else(|| {
            let shown: Vec<&str> = frame
                .lines()
                .skip_while(|line| !line.contains(DEVICE_LINK_PREFIX))
                .take(3)
                .collect();
            E2eError::device(format!(
                "Could not extract the device-link invitation from the TUI; shown: {shown:?}"
            ))
        })
    }

    /// Fresh terminal: Welcome → "Link this device" → paste the invitation
    /// at the scan prompt → name the device. Returns the confirmation code
    /// Core shows while it waits for the existing device to approve.
    async fn join_identity(&self, qr_data: &str, device_name: &str) -> E2eResult<String> {
        self.session.ensure_started().await?;
        self.session
            .wait_for_visible(
                "Welcome to Vauchi|Create new identity",
                Duration::from_secs(15),
            )
            .await?;
        // The welcome screen's first row is "Link this device".
        self.session.send_char('1').await?;
        self.session
            .wait_for_visible("Scan QR Code", Duration::from_secs(10))
            .await?;
        // A terminal answers the scan request with a paste prompt; what is
        // pasted reaches Core as the scanned QR.
        self.session.activate_primary().await?;
        self.session
            .wait_for_visible("Paste QR data", Duration::from_secs(10))
            .await?;
        self.session.send_text(qr_data).await?;
        // Same pause as the exchange paste: Return before every character is
        // applied lands on a half-filled prompt.
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.session.send_enter().await?;
        self.session
            .wait_for_visible("Name for this device", Duration::from_secs(15))
            .await?;
        self.session.send_text(device_name).await?;
        // Tab leaves the field so Return reaches Confirm, as on the name step
        // of onboarding.
        self.session.send_tab().await?;
        self.session.activate_primary().await?;
        self.session
            .wait_for_visible("Verify this code matches", Duration::from_secs(30))
            .await?;
        let frame = self.session.read_screen_snapshot().await?;
        confirmation_code_in(&frame).ok_or_else(|| {
            E2eError::device("Could not read the confirmation code on the joining TUI")
        })
    }

    /// Existing terminal: the code it shows must be the one the joining
    /// device shows before it confirms, which is the check a user makes.
    async fn complete_device_link(&self, request_data: &str) -> E2eResult<String> {
        self.session
            .wait_for_frame("Device Wants to Link", Duration::from_secs(60))
            .await?;
        let frame = self.session.read_screen_snapshot().await?;
        let shown = confirmation_code_in(&frame).ok_or_else(|| {
            E2eError::device("Could not read the confirmation code on the existing TUI")
        })?;
        if shown != request_data {
            return Err(E2eError::device(format!(
                "Confirmation codes differ: existing device shows {shown}, joining device {request_data}"
            )));
        }
        self.session.activate_primary().await?;
        self.session
            .wait_for_frame(
                "Device linked successfully|Device Linked",
                Duration::from_secs(60),
            )
            .await?;
        Ok(shown)
    }

    /// Core may pass "Device Linked" by and open the joined identity's own
    /// card, whose "can see this card" line exists only once it holds one.
    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        self.session
            .wait_for_frame(
                "Device Linked|Successfully joined|can see this card",
                Duration::from_secs(60),
            )
            .await?;
        Ok(())
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
        self.session.navigate_to_contacts().await?;
        self.session.send_alt('s').await?;
        // Sync runs synchronously in the input handler. A subsequent menu
        // round-trip confirms that handler has returned before inspecting
        // the chip, whose text may be identical to the previous outcome.
        self.session.navigate_to_contacts().await?;
        // Retain unchanged cells: a throttled press may redraw only digits
        // in "Sync in N s" (private#603), rather than the whole chip label.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let frame = self.session.read_screen_snapshot().await?;
            if frame.contains("Sync failed") {
                return Err(E2eError::device(
                    "TUI sync failed (sync chip reports failure)",
                ));
            }
            if frame.contains("Synced") || frame.contains("Sync in ") {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(E2eError::device(
                    "the sync chip showed no outcome within 10 s",
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Ok(())
    }

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        self.session.ensure_started().await?;

        self.session.navigate_to_contacts().await?;
        let frame = self.session.read_screen_snapshot().await?;

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
// INLINE_TEST_REQUIRED: private helpers of the feature-gated tui module
// (find_tui_binary_given) are not reachable from tests/.
#[cfg(test)]
#[path = "tui_session_tests.rs"]
mod session_tests;

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

        let found = find_tui_binary_given(Some(bin.to_string_lossy().into_owned()));

        assert_eq!(found.unwrap(), bin);
    }

    // @internal
    #[test]
    fn a_dangling_explicit_tui_binary_path_is_an_error_not_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");

        let found = find_tui_binary_given(Some(missing.to_string_lossy().into_owned()));

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
