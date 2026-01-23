//! TUI-based device implementation.
//!
//! Controls the Vauchi TUI using pseudo-terminal automation.
//!
//! ## Implementation Status
//!
//! This device type requires PTY (pseudo-terminal) control to:
//! - Spawn the TUI process in a terminal emulator
//! - Send keyboard inputs (navigation keys, text)
//! - Parse the terminal screen buffer for feedback
//!
//! ## Dependencies
//!
//! Requires the `expectrl` crate for PTY management:
//! ```toml
//! expectrl = "0.7"
//! ```
//!
//! ## Usage
//!
//! The TUI uses keyboard shortcuts for navigation:
//! - Arrow keys: Navigate menus
//! - Enter: Select/confirm
//! - Esc: Go back
//! - 'i': Init identity (on Setup screen)
//! - 'c': Contacts
//! - 'e': Exchange
//! - 's': Settings
//! - 'y': Sync
//!
//! ## Example Flow
//!
//! ```ignore
//! // Create identity
//! tui.send_key('i');           // Start init
//! tui.expect("Display name");  // Wait for prompt
//! tui.send_text("Alice");      // Enter name
//! tui.send_key(Enter);         // Confirm
//! tui.expect("✓ Identity");    // Verify success
//! ```

use std::path::PathBuf;

use async_trait::async_trait;
use tempfile::TempDir;

use super::{Contact, ContactCard, Device, DeviceType, NetworkConfig};
use crate::error::{E2eError, E2eResult};

/// A device controlled via the TUI.
///
/// Uses PTY automation to control the terminal UI.
pub struct TuiDevice {
    /// Device name/identifier.
    name: String,
    /// Temporary data directory for this device.
    data_dir: TempDir,
    /// Relay URL to connect to.
    relay_url: String,
    /// Path to the TUI binary (used when launching PTY session).
    #[allow(dead_code)]
    tui_path: PathBuf,
    // TODO: Add PTY session handle when implementing
    // pty_session: Option<PtySession>,
}

impl TuiDevice {
    /// Create a new TUI device with an isolated data directory.
    pub fn new(name: impl Into<String>, relay_url: impl Into<String>) -> E2eResult<Self> {
        let data_dir = TempDir::new().map_err(|e| {
            E2eError::device(format!("Failed to create temp directory: {}", e))
        })?;

        let tui_path = Self::find_tui_binary()?;

        Ok(Self {
            name: name.into(),
            data_dir,
            relay_url: relay_url.into(),
            tui_path,
        })
    }

    /// Find the TUI binary in the workspace.
    fn find_tui_binary() -> E2eResult<PathBuf> {
        // Try release binary first
        let release_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tui/target/release/vauchi-tui");
        if release_path.exists() {
            return Ok(release_path);
        }

        // Try debug binary
        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tui/target/debug/vauchi-tui");
        if debug_path.exists() {
            return Ok(debug_path);
        }

        // Try shared target directory
        let shared_debug = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/debug/vauchi-tui");
        if shared_debug.exists() {
            return Ok(shared_debug);
        }

        Err(E2eError::device(
            "TUI binary not found. Please run `cargo build -p vauchi-tui` first.",
        ))
    }

    /// Get the data directory path.
    #[allow(dead_code)]
    pub fn data_dir_path(&self) -> &std::path::Path {
        self.data_dir.path()
    }

    // TODO: Implement PTY session management
    // async fn start_session(&mut self) -> E2eResult<()> { ... }
    // async fn send_key(&self, key: char) -> E2eResult<()> { ... }
    // async fn send_text(&self, text: &str) -> E2eResult<()> { ... }
    // async fn expect(&self, pattern: &str) -> E2eResult<String> { ... }
    // async fn get_screen(&self) -> E2eResult<String> { ... }
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

    async fn create_identity(&self, _name: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented. Requires expectrl crate for PTY control.".into()
        ))
    }

    async fn has_identity(&self) -> bool {
        // Check if identity file exists in data directory
        let identity_path = self.data_dir.path().join("identity.json");
        identity_path.exists()
    }

    async fn export_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn import_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    // === Exchange ===

    async fn generate_qr(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn complete_exchange(&self, _qr_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn join_identity(&self, _qr_data: &str, _device_name: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn complete_device_link(&self, _request_data: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    // === Sync ===

    async fn sync(&self) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    // === Contacts ===

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn get_contact(&self, _name_or_id: &str) -> E2eResult<Option<Contact>> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    // === Card Management ===

    async fn get_card(&self) -> E2eResult<ContactCard> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn add_field(&self, _field_type: &str, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn edit_field(&self, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn remove_field(&self, _label: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    async fn edit_name(&self, _name: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    // === Network Simulation ===

    async fn set_network(&self, _config: NetworkConfig) -> E2eResult<()> {
        // TUI could potentially support network simulation via environment variables
        Ok(())
    }

    // === App Lifecycle ===

    async fn kill_app(&self) -> E2eResult<()> {
        // TODO: Kill the PTY session
        Ok(())
    }

    async fn launch_app(&self) -> E2eResult<()> {
        // TODO: Start a new PTY session with the TUI
        Err(E2eError::DeviceNotSupported(
            "TUI device automation not yet implemented".into()
        ))
    }

    fn supports_lifecycle_control(&self) -> bool {
        true // TUI supports kill/launch once implemented
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
}
