// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Mobile device implementation using Maestro.
//!
//! Controls iOS Simulator and Android Emulator using Maestro CLI.
//!
//! ## What is Maestro?
//!
//! Maestro (https://maestro.mobile.dev/) is a cross-platform mobile UI
//! automation tool that uses declarative YAML flows. It's simpler than
//! Appium and doesn't require code.
//!
//! ## Installation
//!
//! ```bash
//! # macOS/Linux
//! curl -Ls "https://get.maestro.mobile.dev" | bash
//!
//! # Verify installation
//! maestro --version
//! ```
//!
//! ## Requirements
//!
//! ### iOS Simulator
//! - macOS with Xcode installed
//! - iOS Simulator (via `xcrun simctl`)
//! - App built for simulator (`.app` bundle)
//!
//! ### Android Emulator
//! - Android SDK with emulator
//! - ADB in PATH
//! - APK built (debug or release)
//! - AVD created (e.g., `Pixel_7`)
//!
//! ## Flow Structure
//!
//! Maestro flows are YAML files that describe UI interactions:
//!
//! ```yaml
//! # create_identity.yaml
//! appId: com.vauchi  # Android: com.vauchi, iOS: app.vauchi.ios
//! ---
//! - launchApp
//! - tapOn: "Create Identity"
//! - inputText: "Alice"
//! - tapOn: "Continue"
//! - assertVisible: "Identity created"
//! ```
//!
//! ## Directory Structure
//!
//! ```text
//! e2e/maestro/
//! ├── ios/
//! │   ├── create_identity.yaml
//! │   ├── generate_qr.yaml
//! │   ├── complete_exchange.yaml
//! │   ├── sync.yaml
//! │   └── list_contacts.yaml
//! └── android/
//!     ├── create_identity.yaml
//!     ├── generate_qr.yaml
//!     ├── complete_exchange.yaml
//!     ├── sync.yaml
//!     └── list_contacts.yaml
//! ```
//!
//! ## Example Usage
//!
//! ```ignore
//! let device = MaestroDevice::ios("Alice", "iPhone 15 Pro", &relay_url)?;
//! device.launch_app().await?;
//! device.create_identity("Alice").await?;
//! let qr = device.generate_qr().await?;
//! ```

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;

use super::{Contact, ContactCard, Device, DeviceType, NetworkConfig};
use crate::error::{E2eError, E2eResult};

/// Platform for Maestro device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaestroPlatform {
    /// iOS Simulator
    Ios,
    /// Android Emulator
    Android,
}

impl std::fmt::Display for MaestroPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaestroPlatform::Ios => write!(f, "iOS"),
            MaestroPlatform::Android => write!(f, "Android"),
        }
    }
}

/// A mobile device controlled via Maestro.
///
/// Uses Maestro CLI to execute YAML flows that control the mobile app.
pub struct MaestroDevice {
    /// Device name/identifier.
    name: String,
    /// Platform (iOS or Android).
    platform: MaestroPlatform,
    /// Simulator/emulator device name (e.g., "iPhone 15 Pro", "Pixel_7").
    device_name: String,
    /// App bundle ID (iOS) or package name (Android).
    app_id: String,
    /// Relay URL to connect to.
    relay_url: String,
    /// Path to Maestro flows directory.
    flows_dir: PathBuf,
    /// Current network configuration.
    network_config: NetworkConfig,
}

impl MaestroDevice {
    /// Create a new iOS Simulator device.
    pub fn ios(
        name: impl Into<String>,
        simulator_name: impl Into<String>,
        relay_url: impl Into<String>,
    ) -> E2eResult<Self> {
        Self::new(
            name,
            MaestroPlatform::Ios,
            simulator_name,
            "app.vauchi.ios",
            relay_url,
        )
    }

    /// Create a new Android Emulator device.
    pub fn android(
        name: impl Into<String>,
        emulator_name: impl Into<String>,
        relay_url: impl Into<String>,
    ) -> E2eResult<Self> {
        Self::new(
            name,
            MaestroPlatform::Android,
            emulator_name,
            "com.vauchi",
            relay_url,
        )
    }

    /// Create a new Maestro device.
    pub fn new(
        name: impl Into<String>,
        platform: MaestroPlatform,
        device_name: impl Into<String>,
        app_id: impl Into<String>,
        relay_url: impl Into<String>,
    ) -> E2eResult<Self> {
        // Check if Maestro is installed
        Self::check_maestro_installed()?;

        // Determine flows directory
        let flows_dir = Self::find_flows_dir(platform)?;

        Ok(Self {
            name: name.into(),
            platform,
            device_name: device_name.into(),
            app_id: app_id.into(),
            relay_url: relay_url.into(),
            flows_dir,
            network_config: NetworkConfig::default(),
        })
    }

    /// Check if Maestro CLI is installed.
    fn check_maestro_installed() -> E2eResult<()> {
        // Check if maestro binary exists in PATH
        match std::process::Command::new("maestro")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Ok(status) if status.success() => Ok(()),
            _ => Err(E2eError::DeviceNotSupported(
                "Maestro CLI not found. Install with: curl -Ls 'https://get.maestro.mobile.dev' | bash".into()
            ))
        }
    }

    /// Find the Maestro flows directory.
    fn find_flows_dir(platform: MaestroPlatform) -> E2eResult<PathBuf> {
        let subdir = match platform {
            MaestroPlatform::Ios => "ios",
            MaestroPlatform::Android => "android",
        };

        let flows_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("maestro")
            .join(subdir);

        // Don't require the directory to exist yet - we're just setting up the path
        Ok(flows_dir)
    }

    /// Return the Maestro `--platform` flag value for this device.
    fn platform_flag(&self) -> &'static str {
        match self.platform {
            MaestroPlatform::Ios => "ios",
            MaestroPlatform::Android => "android",
        }
    }

    /// Run a Maestro flow with optional extra environment variables.
    ///
    /// Passes `--platform` explicitly to avoid XCTest driver timeout on iOS
    /// (where Maestro may attempt to connect to an Android device first) and
    /// to ensure correct device targeting on Android when both an emulator and
    /// a simulator are running simultaneously.
    async fn run_flow(&self, flow_name: &str, env_vars: &[(&str, &str)]) -> E2eResult<String> {
        let flow_path = self.flows_dir.join(format!("{}.yaml", flow_name));

        if !flow_path.exists() {
            return Err(E2eError::device(format!(
                "Maestro flow not found: {}. Create it at: {}",
                flow_name,
                flow_path.display()
            )));
        }

        let mut cmd = Command::new("maestro");
        cmd.arg("test")
            .arg(&flow_path)
            .arg("--platform")
            .arg(self.platform_flag())
            .arg("--device")
            .arg(&self.device_name)
            .env("MAESTRO_APP_ID", &self.app_id)
            .env("VAUCHI_RELAY_URL", &self.relay_url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in env_vars {
            cmd.env(key, value);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| E2eError::device(format!("Failed to run Maestro flow: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(E2eError::device(format!(
                "Maestro flow '{}' failed: {}",
                flow_name, stderr
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Read QR data written by the generate_qr flow.
    fn read_qr_data(&self) -> E2eResult<String> {
        std::fs::read_to_string("/tmp/vauchi_qr_data.txt")
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                E2eError::device(format!(
                    "Failed to read QR data from /tmp/vauchi_qr_data.txt: {}",
                    e
                ))
            })
    }

    /// Read contacts list written by the list_contacts flow.
    fn read_contacts_file(&self) -> E2eResult<Vec<Contact>> {
        let content = std::fs::read_to_string("/tmp/vauchi_contacts.txt").map_err(|e| {
            E2eError::device(format!(
                "Failed to read contacts from /tmp/vauchi_contacts.txt: {}",
                e
            ))
        })?;

        Ok(content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|name| Contact {
                name: name.trim().to_string(),
                id: None,
                verified: false,
            })
            .collect())
    }

    /// Read card data written by the get_card flow.
    fn read_card_file(&self) -> E2eResult<ContactCard> {
        let content = std::fs::read_to_string("/tmp/vauchi_card.json").map_err(|e| {
            E2eError::device(format!(
                "Failed to read card from /tmp/vauchi_card.json: {}",
                e
            ))
        })?;

        serde_json::from_str(&content)
            .map_err(|e| E2eError::device(format!("Failed to parse card JSON: {}", e)))
    }

    /// Run a platform-specific app lifecycle command.
    async fn run_app_command(&self, action: &str) -> E2eResult<()> {
        let output = match self.platform {
            MaestroPlatform::Ios => {
                let args = match action {
                    "launch" => vec!["simctl", "launch", "booted", &self.app_id],
                    "terminate" => vec!["simctl", "terminate", "booted", &self.app_id],
                    _ => return Err(E2eError::device(format!("Unknown action: {}", action))),
                };
                Command::new("xcrun")
                    .args(&args)
                    .output()
                    .await
                    .map_err(|e| E2eError::device(format!("xcrun failed: {}", e)))?
            }
            MaestroPlatform::Android => {
                let activity = format!("{}/{}.MainActivity", self.app_id, self.app_id);
                let args: Vec<&str> = match action {
                    "launch" => vec!["shell", "am", "start", "-n", &activity],
                    "terminate" => vec!["shell", "am", "force-stop", &self.app_id],
                    _ => return Err(E2eError::device(format!("Unknown action: {}", action))),
                };
                Command::new("adb")
                    .args(&args)
                    .output()
                    .await
                    .map_err(|e| E2eError::device(format!("adb failed: {}", e)))?
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(E2eError::device(format!(
                "App {} failed: {}",
                action, stderr
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl Device for MaestroDevice {
    fn device_type(&self) -> DeviceType {
        match self.platform {
            MaestroPlatform::Ios => DeviceType::IosSimulator,
            MaestroPlatform::Android => DeviceType::AndroidEmulator,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn relay_url(&self) -> &str {
        &self.relay_url
    }

    // === Identity Management ===

    async fn create_identity(&self, name: &str) -> E2eResult<()> {
        self.run_flow("create_identity", &[("NAME", name)]).await?;
        Ok(())
    }

    async fn has_identity(&self) -> bool {
        // Try getting the card — if it works, identity exists
        self.run_flow("get_card", &[("CONTACT_NAME", "Your Card")])
            .await
            .is_ok()
    }

    async fn export_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: export_identity not supported (mobile uses cloud backup)".into(),
        ))
    }

    async fn import_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: import_identity not supported (mobile uses cloud backup)".into(),
        ))
    }

    // === Exchange ===

    async fn generate_qr(&self) -> E2eResult<String> {
        self.run_flow("generate_qr", &[]).await?;
        self.read_qr_data()
    }

    async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()> {
        self.run_flow("complete_exchange", &[("QR_DATA", qr_data)])
            .await?;
        Ok(())
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        self.run_flow("link_device", &[]).await?;
        // Link data is written by the flow; read it from the temp file
        std::fs::read_to_string("/tmp/vauchi_link_data.txt")
            .map(|s| s.trim().to_string())
            .map_err(|e| E2eError::device(format!("Failed to read link data: {}", e)))
    }

    async fn join_identity(&self, _qr_data: &str, _device_name: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: join_identity requires device-specific flow".into(),
        ))
    }

    async fn complete_device_link(&self, _request_data: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: complete_device_link requires device-specific flow".into(),
        ))
    }

    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: finish_device_join requires device-specific flow".into(),
        ))
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: list_devices requires device-specific flow".into(),
        ))
    }

    // === Sync ===

    async fn sync(&self) -> E2eResult<()> {
        self.run_flow("sync", &[]).await?;
        Ok(())
    }

    // === Contacts ===

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        self.run_flow("list_contacts", &[]).await?;
        self.read_contacts_file()
    }

    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>> {
        self.run_flow("get_card", &[("CONTACT_NAME", name_or_id)])
            .await?;
        let card = self.read_card_file()?;
        if card.name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Contact {
                name: card.name,
                id: None,
                verified: false,
            }))
        }
    }

    // === Card Management ===

    async fn get_card(&self) -> E2eResult<ContactCard> {
        self.run_flow("get_card", &[("CONTACT_NAME", "Your Card")])
            .await?;
        self.read_card_file()
    }

    async fn add_field(&self, field_type: &str, label: &str, value: &str) -> E2eResult<()> {
        self.run_flow(
            "add_field",
            &[
                ("FIELD_TYPE", field_type),
                ("FIELD_LABEL", label),
                ("FIELD_VALUE", value),
            ],
        )
        .await?;
        Ok(())
    }

    async fn edit_field(&self, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: edit_field requires dedicated flow".into(),
        ))
    }

    async fn remove_field(&self, _label: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: remove_field requires dedicated flow".into(),
        ))
    }

    async fn edit_name(&self, _name: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: edit_name requires dedicated flow".into(),
        ))
    }

    // === Network Simulation ===

    async fn set_network(&self, config: NetworkConfig) -> E2eResult<()> {
        let _ = config;
        Ok(())
    }

    fn network_config(&self) -> NetworkConfig {
        self.network_config.clone()
    }

    // === App Lifecycle ===

    async fn background_app(&self) -> E2eResult<()> {
        match self.platform {
            MaestroPlatform::Ios => {
                Command::new("xcrun")
                    .args(["simctl", "ui", "booted", "home"])
                    .output()
                    .await
                    .map_err(|e| E2eError::device(format!("Failed to background app: {}", e)))?;
                Ok(())
            }
            MaestroPlatform::Android => {
                Command::new("adb")
                    .args(["shell", "input", "keyevent", "KEYCODE_HOME"])
                    .output()
                    .await
                    .map_err(|e| E2eError::device(format!("Failed to background app: {}", e)))?;
                Ok(())
            }
        }
    }

    async fn foreground_app(&self) -> E2eResult<()> {
        self.run_app_command("launch").await
    }

    async fn kill_app(&self) -> E2eResult<()> {
        self.run_app_command("terminate").await
    }

    async fn launch_app(&self) -> E2eResult<()> {
        self.run_app_command("launch").await
    }

    // === Proximity Verification ===

    async fn start_proximity_verification(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: proximity verification not automatable in simulator".into(),
        ))
    }

    async fn verify_proximity(&self, _challenge: &str) -> E2eResult<bool> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: proximity verification not automatable in simulator".into(),
        ))
    }

    // === Capabilities ===

    fn supports_network_simulation(&self) -> bool {
        false // Network simulation requires manual setup
    }

    fn supports_lifecycle_control(&self) -> bool {
        true
    }

    fn supports_proximity_verification(&self) -> bool {
        false // Not automatable in simulator
    }
}

// INLINE_TEST_REQUIRED: Tests need access to private methods (platform_flag, find_flows_dir, check_maestro_installed)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maestro_platform_display() {
        assert_eq!(format!("{}", MaestroPlatform::Ios), "iOS");
        assert_eq!(format!("{}", MaestroPlatform::Android), "Android");
    }

    #[test]
    fn test_maestro_platform_flag_ios_returns_ios() {
        // Check if Maestro is actually installed (needed to construct device)
        let maestro_installed = std::process::Command::new("maestro")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !maestro_installed {
            // Skip if Maestro not installed — tested via find_flows_dir path below
            return;
        }

        let device = MaestroDevice::ios("test", "iPhone 15 Pro", "ws://localhost:8080").unwrap();
        assert_eq!(device.platform_flag(), "ios");
    }

    #[test]
    fn test_maestro_platform_flag_android_returns_android() {
        let maestro_installed = std::process::Command::new("maestro")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !maestro_installed {
            return;
        }

        let device = MaestroDevice::android("test", "Pixel_7", "ws://localhost:8080").unwrap();
        assert_eq!(device.platform_flag(), "android");
    }

    #[test]
    fn test_maestro_find_flows_dir_ios_contains_ios_subdir() {
        let flows_dir = MaestroDevice::find_flows_dir(MaestroPlatform::Ios).unwrap();
        assert!(
            flows_dir.ends_with("maestro/ios"),
            "Expected flows dir to end with maestro/ios, got: {}",
            flows_dir.display()
        );
    }

    #[test]
    fn test_maestro_find_flows_dir_android_contains_android_subdir() {
        let flows_dir = MaestroDevice::find_flows_dir(MaestroPlatform::Android).unwrap();
        assert!(
            flows_dir.ends_with("maestro/android"),
            "Expected flows dir to end with maestro/android, got: {}",
            flows_dir.display()
        );
    }

    #[test]
    fn test_maestro_device_creation_depends_on_cli() {
        // Device creation depends on whether Maestro CLI is installed
        let result = MaestroDevice::ios("test", "iPhone 15 Pro", "ws://localhost:8080");

        // Check if Maestro is actually installed
        let maestro_installed = std::process::Command::new("maestro")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if maestro_installed {
            // If Maestro is installed, device creation should succeed
            assert!(result.is_ok(), "Expected Ok when Maestro is installed");
        } else {
            // If Maestro is not installed, device creation should fail
            assert!(
                result.is_err(),
                "Expected Err when Maestro is not installed"
            );
        }
    }
}
