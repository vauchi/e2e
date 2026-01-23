//! Desktop (Tauri) device implementation.
//!
//! Controls the Vauchi desktop app built with Tauri via a test HTTP server.
//!
//! ## Implementation
//!
//! The Tauri app exposes a test HTTP server when VAUCHI_TEST_PORT is set.
//! This allows E2E tests to invoke Tauri commands via REST API:
//!
//! - `GET /health` - Health check
//! - `GET /identity` - Get identity info
//! - `POST /identity` - Create identity
//! - `GET /card` - Get contact card
//! - `GET /contacts` - List contacts
//! - `POST /sync` - Sync with relay
//!
//! ## Requirements
//!
//! - Desktop app binary built (`cargo build -p vauchi-desktop --release`)
//! - xvfb-run for headless testing on Linux

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tempfile::TempDir;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::{CardField, Contact, ContactCard, Device, DeviceType};
use crate::error::{E2eError, E2eResult};

/// Base port for test server (will try sequential ports if busy).
const TEST_PORT_BASE: u16 = 19000;

/// A device controlled via the Tauri desktop app.
///
/// Uses process management and HTTP API for control.
pub struct TauriDevice {
    /// Device name/identifier.
    name: String,
    /// Temporary data directory for this device.
    data_dir: TempDir,
    /// Relay URL to connect to.
    relay_url: String,
    /// Path to the desktop app binary.
    app_path: PathBuf,
    /// Running app process handle (interior mutability for &self methods).
    process: Mutex<Option<Child>>,
    /// Test server port.
    test_port: Mutex<Option<u16>>,
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
            process: Mutex::new(None),
            test_port: Mutex::new(None),
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
            "Desktop app binary not found. Please run `cargo build -p vauchi-desktop --release` first.",
        ))
    }

    /// Get the data directory path.
    #[allow(dead_code)]
    pub fn data_dir_path(&self) -> &std::path::Path {
        self.data_dir.path()
    }

    /// Check if the app process is running.
    pub async fn is_running(&self) -> bool {
        self.process.lock().await.is_some()
    }

    /// Get the test server URL.
    async fn test_url(&self, path: &str) -> E2eResult<String> {
        let port = self.test_port.lock().await;
        let port = port.ok_or_else(|| E2eError::device("App not launched - no test port"))?;
        Ok(format!("http://127.0.0.1:{}{}", port, path))
    }

    /// Make a GET request to the test server.
    async fn get(&self, path: &str) -> E2eResult<serde_json::Value> {
        let url = self.test_url(path).await?;
        let response = reqwest::get(&url)
            .await
            .map_err(|e| E2eError::device(format!("HTTP request failed: {}", e)))?;

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| E2eError::device(format!("Failed to parse JSON: {}", e)))?;

        Ok(json)
    }

    /// Make a POST request to the test server.
    async fn post(&self, path: &str, body: serde_json::Value) -> E2eResult<serde_json::Value> {
        let url = self.test_url(path).await?;
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| E2eError::device(format!("HTTP request failed: {}", e)))?;

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| E2eError::device(format!("Failed to parse JSON: {}", e)))?;

        Ok(json)
    }

    /// Find an available port for the test server.
    fn find_available_port() -> u16 {
        // Use a simple incrementing strategy with randomization
        let base = TEST_PORT_BASE + (std::process::id() as u16 % 1000);
        for offset in 0..100 {
            let port = base + offset;
            if std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
                return port;
            }
        }
        // Fallback to random port
        TEST_PORT_BASE + (rand::random::<u16>() % 1000)
    }

    /// Wait for the test server to be ready.
    async fn wait_for_test_server(&self, port: u16) -> E2eResult<()> {
        let url = format!("http://127.0.0.1:{}/health", port);

        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(200)).await;

            if let Ok(response) = reqwest::get(&url).await {
                if response.status().is_success() {
                    return Ok(());
                }
            }
        }

        Err(E2eError::device("Test server did not become ready"))
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

    async fn create_identity(&self, name: &str) -> E2eResult<()> {
        let body = serde_json::json!({ "name": name });
        let response = self.post("/identity", body).await?;

        if response.get("error").is_some() {
            return Err(E2eError::device(format!(
                "Failed to create identity: {}",
                response["error"]
            )));
        }

        Ok(())
    }

    async fn has_identity(&self) -> bool {
        if let Ok(response) = self.get("/identity").await {
            response["has_identity"].as_bool().unwrap_or(false)
        } else {
            false
        }
    }

    async fn export_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop identity export not yet implemented via test API".into()
        ))
    }

    async fn import_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop identity import not yet implemented via test API".into()
        ))
    }

    // === Exchange ===

    async fn generate_qr(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop QR generation not yet implemented via test API".into()
        ))
    }

    async fn complete_exchange(&self, _qr_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop exchange not yet implemented via test API".into()
        ))
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device linking not yet implemented via test API".into()
        ))
    }

    async fn join_identity(&self, _qr_data: &str, _device_name: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device linking not yet implemented via test API".into()
        ))
    }

    async fn complete_device_link(&self, _request_data: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device linking not yet implemented via test API".into()
        ))
    }

    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device linking not yet implemented via test API".into()
        ))
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        Err(E2eError::DeviceNotSupported(
            "Desktop device listing not yet implemented via test API".into()
        ))
    }

    // === Sync ===

    async fn sync(&self) -> E2eResult<()> {
        let response = self.post("/sync", serde_json::json!({})).await?;

        if let Some(error) = response.get("error") {
            if !error.is_null() {
                return Err(E2eError::device(format!("Sync failed: {}", error)));
            }
        }

        Ok(())
    }

    // === Contacts ===

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        let response = self.get("/contacts").await?;

        let contacts = response["contacts"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|c| Contact {
                name: c["display_name"].as_str().unwrap_or("").to_string(),
                id: c["id"].as_str().map(|s| s.to_string()),
                verified: c["verified"].as_bool().unwrap_or(false),
            })
            .collect();

        Ok(contacts)
    }

    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>> {
        let contacts = self.list_contacts().await?;
        Ok(contacts.into_iter().find(|c| {
            c.name.contains(name_or_id) || c.id.as_ref().map(|id| id.contains(name_or_id)).unwrap_or(false)
        }))
    }

    // === Card Management ===

    async fn get_card(&self) -> E2eResult<ContactCard> {
        let response = self.get("/card").await?;

        let fields = response["fields"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|f| CardField {
                field_type: f["type"].as_str().unwrap_or("").to_string(),
                label: f["label"].as_str().unwrap_or("").to_string(),
                value: f["value"].as_str().unwrap_or("").to_string(),
            })
            .collect();

        Ok(ContactCard {
            name: response["display_name"].as_str().unwrap_or("").to_string(),
            fields,
        })
    }

    async fn add_field(&self, _field_type: &str, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop field management not yet implemented via test API".into()
        ))
    }

    async fn edit_field(&self, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop field management not yet implemented via test API".into()
        ))
    }

    async fn remove_field(&self, _label: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop field management not yet implemented via test API".into()
        ))
    }

    async fn edit_name(&self, _name: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Desktop name editing not yet implemented via test API".into()
        ))
    }

    // === App Lifecycle ===

    async fn kill_app(&self) -> E2eResult<()> {
        let mut process_guard = self.process.lock().await;
        let mut port_guard = self.test_port.lock().await;

        if let Some(mut child) = process_guard.take() {
            // Try graceful kill first
            child.kill().await.map_err(|e| {
                E2eError::device(format!("Failed to kill desktop app: {}", e))
            })?;

            // Wait for process to exit
            let _ = child.wait().await;
        }

        *port_guard = None;
        Ok(())
    }

    async fn launch_app(&self) -> E2eResult<()> {
        let mut process_guard = self.process.lock().await;
        let mut port_guard = self.test_port.lock().await;

        if process_guard.is_some() {
            return Err(E2eError::device("App is already running"));
        }

        // Find available port for test server
        let test_port = Self::find_available_port();

        // Build command - use xvfb-run on Linux for headless operation
        #[cfg(target_os = "linux")]
        let mut cmd = {
            let mut cmd = Command::new("xvfb-run");
            cmd.arg("-a");
            cmd.arg(&self.app_path);
            cmd
        };

        #[cfg(not(target_os = "linux"))]
        let mut cmd = Command::new(&self.app_path);

        cmd.env("VAUCHI_DATA_DIR", self.data_dir.path())
            .env("VAUCHI_RELAY_URL", &self.relay_url)
            .env("VAUCHI_TEST_PORT", test_port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let child = cmd.spawn().map_err(|e| {
            E2eError::device(format!("Failed to launch desktop app: {}", e))
        })?;

        // Store the process handle and port
        *process_guard = Some(child);
        *port_guard = Some(test_port);

        // Release locks before waiting
        drop(process_guard);
        drop(port_guard);

        // Wait for test server to be ready
        self.wait_for_test_server(test_port).await?;

        Ok(())
    }

    fn supports_lifecycle_control(&self) -> bool {
        true
    }
}

impl Drop for TauriDevice {
    fn drop(&mut self) {
        // Kill the app process if still running
        // We can't use async here, so use blocking approach
        if let Ok(mut guard) = self.process.try_lock() {
            if let Some(mut process) = guard.take() {
                let _ = process.start_kill();
            }
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

    #[tokio::test]
    async fn test_is_running_initially_false() {
        if let Ok(device) = TauriDevice::new("test", "ws://localhost:8080") {
            assert!(!device.is_running().await);
        }
    }
}
