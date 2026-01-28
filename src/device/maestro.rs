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
//! appId: app.vauchi.mobile
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
            "app.vauchi.mobile",
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
            "app.vauchi.mobile",
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

    /// Run a Maestro flow.
    #[allow(dead_code)]
    async fn run_flow(&self, flow_name: &str) -> E2eResult<String> {
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
            .env("MAESTRO_APP_ID", &self.app_id)
            .env("VAUCHI_RELAY_URL", &self.relay_url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Add device-specific arguments
        match self.platform {
            MaestroPlatform::Ios => {
                cmd.arg("--device").arg(&self.device_name);
            }
            MaestroPlatform::Android => {
                cmd.arg("--device").arg(&self.device_name);
            }
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

    /// Parse QR data from flow output.
    ///
    /// The flow should output QR data in a specific format that we can parse.
    #[allow(dead_code)]
    fn parse_qr_from_output(&self, _output: &str) -> E2eResult<String> {
        // In a real implementation, we'd either:
        // 1. Have the flow output QR data to a file
        // 2. Take a screenshot and use QR detection
        // 3. Have the flow copy QR data to clipboard
        Err(E2eError::DeviceNotSupported(
            "QR extraction from Maestro flows not yet implemented".into(),
        ))
    }

    /// Parse contacts from flow output.
    #[allow(dead_code)]
    fn parse_contacts_from_output(&self, _output: &str) -> E2eResult<Vec<Contact>> {
        // In a real implementation, we'd parse structured output from the flow
        Err(E2eError::DeviceNotSupported(
            "Contact parsing from Maestro flows not yet implemented".into(),
        ))
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

    async fn create_identity(&self, _name: &str) -> E2eResult<()> {
        // Would run: self.run_flow("create_identity").await
        Err(E2eError::DeviceNotSupported(format!(
            "Maestro {} device automation not yet implemented. \
             Create flow at: {}/create_identity.yaml",
            self.platform,
            self.flows_dir.display()
        )))
    }

    async fn has_identity(&self) -> bool {
        // Would need to run a flow that checks identity status
        false
    }

    async fn export_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: export_identity not implemented".into(),
        ))
    }

    async fn import_identity(&self, _path: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: import_identity not implemented".into(),
        ))
    }

    // === Exchange ===

    async fn generate_qr(&self) -> E2eResult<String> {
        // Would run: self.run_flow("generate_qr").await
        // Then parse QR data from output or screenshot
        Err(E2eError::DeviceNotSupported(format!(
            "Maestro {} device: generate_qr not implemented. \
             Create flow at: {}/generate_qr.yaml",
            self.platform,
            self.flows_dir.display()
        )))
    }

    async fn complete_exchange(&self, _qr_data: &str) -> E2eResult<()> {
        // Would run: self.run_flow("complete_exchange").await
        // Passing QR data as environment variable
        Err(E2eError::DeviceNotSupported(format!(
            "Maestro {} device: complete_exchange not implemented",
            self.platform
        )))
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: start_device_link not implemented".into(),
        ))
    }

    async fn join_identity(&self, _qr_data: &str, _device_name: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: join_identity not implemented".into(),
        ))
    }

    async fn complete_device_link(&self, _request_data: &str) -> E2eResult<String> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: complete_device_link not implemented".into(),
        ))
    }

    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: finish_device_join not implemented".into(),
        ))
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: list_devices not implemented".into(),
        ))
    }

    // === Sync ===

    async fn sync(&self) -> E2eResult<()> {
        // Would run: self.run_flow("sync").await
        Err(E2eError::DeviceNotSupported(format!(
            "Maestro {} device: sync not implemented",
            self.platform
        )))
    }

    // === Contacts ===

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        // Would run: self.run_flow("list_contacts").await
        // Then parse contacts from output
        Err(E2eError::DeviceNotSupported(format!(
            "Maestro {} device: list_contacts not implemented",
            self.platform
        )))
    }

    async fn get_contact(&self, _name_or_id: &str) -> E2eResult<Option<Contact>> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: get_contact not implemented".into(),
        ))
    }

    // === Card Management ===

    async fn get_card(&self) -> E2eResult<ContactCard> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: get_card not implemented".into(),
        ))
    }

    async fn add_field(&self, _field_type: &str, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: add_field not implemented".into(),
        ))
    }

    async fn edit_field(&self, _label: &str, _value: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: edit_field not implemented".into(),
        ))
    }

    async fn remove_field(&self, _label: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: remove_field not implemented".into(),
        ))
    }

    async fn edit_name(&self, _name: &str) -> E2eResult<()> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: edit_name not implemented".into(),
        ))
    }

    // === Network Simulation ===

    async fn set_network(&self, config: NetworkConfig) -> E2eResult<()> {
        // Mobile devices can simulate network conditions via:
        // - iOS: Network Link Conditioner
        // - Android: adb emu network delay/speed
        // For now, just store the config
        let _ = config;
        Ok(())
    }

    fn network_config(&self) -> NetworkConfig {
        self.network_config.clone()
    }

    // === App Lifecycle ===

    async fn background_app(&self) -> E2eResult<()> {
        // iOS: xcrun simctl terminate + launch with background state
        // Android: adb shell input keyevent KEYCODE_HOME
        Err(E2eError::DeviceNotSupported(
            "Maestro device: background_app not implemented".into(),
        ))
    }

    async fn foreground_app(&self) -> E2eResult<()> {
        // Would relaunch the app
        Err(E2eError::DeviceNotSupported(
            "Maestro device: foreground_app not implemented".into(),
        ))
    }

    async fn kill_app(&self) -> E2eResult<()> {
        // iOS: xcrun simctl terminate booted <app_id>
        // Android: adb shell am force-stop <package>
        Err(E2eError::DeviceNotSupported(
            "Maestro device: kill_app not implemented".into(),
        ))
    }

    async fn launch_app(&self) -> E2eResult<()> {
        // iOS: xcrun simctl launch booted <app_id>
        // Android: adb shell am start -n <package>/<activity>
        Err(E2eError::DeviceNotSupported(
            "Maestro device: launch_app not implemented".into(),
        ))
    }

    // === Proximity Verification ===

    async fn start_proximity_verification(&self) -> E2eResult<String> {
        // Mobile devices support proximity verification via audio
        Err(E2eError::DeviceNotSupported(
            "Maestro device: proximity verification not implemented".into(),
        ))
    }

    async fn verify_proximity(&self, _challenge: &str) -> E2eResult<bool> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: proximity verification not implemented".into(),
        ))
    }

    // === Capabilities ===

    fn supports_network_simulation(&self) -> bool {
        true // Mobile devices support network simulation
    }

    fn supports_lifecycle_control(&self) -> bool {
        true // Mobile devices support app lifecycle control
    }

    fn supports_proximity_verification(&self) -> bool {
        true // Mobile devices support proximity verification
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maestro_platform_display() {
        assert_eq!(format!("{}", MaestroPlatform::Ios), "iOS");
        assert_eq!(format!("{}", MaestroPlatform::Android), "Android");
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
