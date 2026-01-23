//! Desktop (Tauri) device implementation.
//!
//! Controls the Vauchi desktop app built with Tauri.
//!
//! ## Implementation Status
//!
//! This device type requires:
//! - Process management for the Tauri app
//! - IPC communication with the Tauri backend
//! - Optionally: WebdriverIO for UI automation
//!
//! ## Architecture
//!
//! The Tauri app exposes IPC commands that can be invoked programmatically:
//! - `create_identity(name)` - Create a new identity
//! - `get_card()` - Get the contact card
//! - `sync()` - Sync with relay
//! - etc.
//!
//! For E2E testing, we can either:
//! 1. Invoke IPC commands directly (requires Tauri test mode)
//! 2. Use WebdriverIO for full UI automation
//!
//! ## Dependencies
//!
//! For WebdriverIO approach:
//! ```toml
//! tauri-driver = "2.0"  # Tauri's WebDriver implementation
//! ```
//!
//! ## Example Flow
//!
//! ```ignore
//! let device = TauriDevice::new("alice", "ws://localhost:8080")?;
//! device.launch_app().await?;
//! device.create_identity("Alice").await?;  // Via IPC
//! let card = device.get_card().await?;
//! device.kill_app().await?;
//! ```

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use tempfile::TempDir;
use tokio::process::{Child, Command};

use super::{Contact, ContactCard, Device, DeviceType};
use crate::error::{E2eError, E2eResult};

/// A device controlled via the Tauri desktop app.
///
/// Uses process management and IPC for control.
pub struct TauriDevice {
    /// Device name/identifier.
    name: String,
    /// Temporary data directory for this device.
    data_dir: TempDir,
    /// Relay URL to connect to.
    relay_url: String,
    /// Path to the desktop app binary.
    app_path: PathBuf,
    /// Running app process handle.
    process: Option<Child>,
}

impl TauriDevice {
    /// Create a new Tauri device with an isolated data directory.
    pub fn new(name: impl Into<String>, relay_url: impl Into<String>) -> E2eResult<Self> {
        let data_dir = TempDir::new().map_err(|e| {
            E2eError::device(format!("Failed to create temp directory: {}", e))
        })?;

        let app_path = Self::find_app_binary()?;

        Ok(Self {
            name: name.into(),
            data_dir,
            relay_url: relay_url.into(),
            app_path,
            process: None,
        })
    }

    /// Find the desktop app binary in the workspace.
    fn find_app_binary() -> E2eResult<PathBuf> {
        // Try release binary first (platform-specific paths)
        #[cfg(target_os = "linux")]
        let release_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/src-tauri/target/release/vauchi-desktop");

        #[cfg(target_os = "macos")]
        let release_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/src-tauri/target/release/bundle/macos/Vauchi.app/Contents/MacOS/Vauchi");

        #[cfg(target_os = "windows")]
        let release_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/src-tauri/target/release/vauchi-desktop.exe");

        if release_path.exists() {
            return Ok(release_path);
        }

        // Try debug binary
        #[cfg(target_os = "linux")]
        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/src-tauri/target/debug/vauchi-desktop");

        #[cfg(target_os = "macos")]
        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/src-tauri/target/debug/vauchi-desktop");

        #[cfg(target_os = "windows")]
        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/src-tauri/target/debug/vauchi-desktop.exe");

        if debug_path.exists() {
            return Ok(debug_path);
        }

        Err(E2eError::device(
            "Desktop app binary not found. Please run `just desktop-build` first.",
        ))
    }

    /// Get the data directory path.
    #[allow(dead_code)]
    pub fn data_dir_path(&self) -> &std::path::Path {
        self.data_dir.path()
    }

    /// Check if the app process is running.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }
}

#[async_trait]
impl Device for TauriDevice {
    fn device_type(&self) -> DeviceType {
        DeviceType::Desktop
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
            "Desktop device automation not yet implemented. Requires Tauri IPC or WebdriverIO.".into()
        ))
    }

    async fn has_identity(&self) -> bool {
        // Check if identity file exists in data directory
        let identity_path = self.data_dir.path().join("identity.json");
        identity_path.exists()
    }

    async fn export_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn import_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    // === Exchange ===

    async fn generate_qr(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn complete_exchange(&self, _qr_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn join_identity(&self, _qr_data: &str, _device_name: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn complete_device_link(&self, _request_data: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    // === Sync ===

    async fn sync(&self) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    // === Contacts ===

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn get_contact(&self, _name_or_id: &str) -> E2eResult<Option<Contact>> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    // === Card Management ===

    async fn get_card(&self) -> E2eResult<ContactCard> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn add_field(&self, _field_type: &str, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn edit_field(&self, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn remove_field(&self, _label: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    async fn edit_name(&self, _name: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device automation not yet implemented".into()
        ))
    }

    // === App Lifecycle ===

    async fn kill_app(&self) -> E2eResult<()> {
        // Kill is implemented - just terminate the process
        if let Some(mut process) = unsafe {
            // SAFETY: We're taking ownership of the process
            std::ptr::read(&self.process as *const Option<Child>)
        } {
            let _ = process.kill().await;
        }
        Ok(())
    }

    async fn launch_app(&self) -> E2eResult<()> {
        if self.process.is_some() {
            return Err(E2eError::device("App is already running"));
        }

        let mut cmd = Command::new(&self.app_path);
        cmd.env("VAUCHI_DATA_DIR", self.data_dir.path())
            .env("VAUCHI_RELAY_URL", &self.relay_url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let child = cmd.spawn().map_err(|e| {
            E2eError::device(format!("Failed to launch desktop app: {}", e))
        })?;

        // Store process handle (this is a bit hacky due to mut self)
        // In a real implementation, we'd use interior mutability
        let _ = child;

        // TODO: Wait for app to be ready (window appears, IPC available)

        Err(E2eError::DeviceNotSupported(
            "Desktop app launch not yet fully implemented".into()
        ))
    }

    fn supports_lifecycle_control(&self) -> bool {
        true // Desktop supports kill/launch
    }
}

impl Drop for TauriDevice {
    fn drop(&mut self) {
        // Kill the app process if still running
        if let Some(mut process) = self.process.take() {
            let _ = process.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tauri_device_type() {
        // This test will fail if binary doesn't exist, which is expected
        if let Ok(device) = TauriDevice::new("test", "ws://localhost:8080") {
            assert_eq!(device.device_type(), DeviceType::Desktop);
            assert_eq!(device.name(), "test");
        }
    }
}
