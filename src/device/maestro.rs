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
//! Select a specific release from the
//! [official releases](https://github.com/mobile-dev-inc/Maestro/releases),
//! verify the signed release and inspect the archive, then add its `bin`
//! directory to `PATH`. Confirm the reviewed version with `maestro --version`.
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
//! appId: app.vauchi  # Android: app.vauchi, iOS: app.vauchi.ios
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

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;

use super::maestro_artifacts;
use super::{Contact, ContactCard, Device, DeviceType, NetworkConfig};
use crate::error::{E2eError, E2eResult};

const MAESTRO_INSTALL_HELP: &str = "Install a reviewed, pinned release from https://github.com/mobile-dev-inc/Maestro/releases and verify it with maestro --version";

/// Platform for Maestro device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaestroPlatform {
    /// iOS Simulator
    Ios,
    /// Android Emulator
    Android,
}

impl MaestroPlatform {
    /// The app the platform's flows drive; each flow names it as `appId`.
    fn app_id(self) -> &'static str {
        match self {
            MaestroPlatform::Ios => "app.vauchi.ios",
            MaestroPlatform::Android => "app.vauchi",
        }
    }
}

impl std::fmt::Display for MaestroPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaestroPlatform::Ios => write!(f, "iOS"),
            MaestroPlatform::Android => write!(f, "Android"),
        }
    }
}

/// The `takeScreenshot` names the QR flows save under.
const EXCHANGE_QR_SCREENSHOT: &str = "exchange_qr";
const DEVICE_LINK_QR_SCREENSHOT: &str = "device_link_qr";

#[derive(Clone, Copy)]
enum MaestroOutput {
    Contacts,
    Card,
}

impl MaestroOutput {
    fn file_name(self) -> &'static str {
        match self {
            Self::Contacts => "contacts.txt",
            Self::Card => "card.json",
        }
    }
}

struct MaestroWorkspace {
    directory: tempfile::TempDir,
}

impl MaestroWorkspace {
    fn new() -> E2eResult<Self> {
        let directory = tempfile::Builder::new()
            .prefix("vauchi-maestro-")
            .tempdir()
            .map_err(|e| E2eError::device(format!("Failed to create Maestro workspace: {e}")))?;
        Ok(Self { directory })
    }

    fn output_path(&self, output: MaestroOutput) -> PathBuf {
        self.directory.path().join(output.file_name())
    }

    fn prepare_output(&self, output: MaestroOutput) -> E2eResult<PathBuf> {
        let path = self.output_path(output);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(E2eError::device(format!(
                    "Failed to clear stale Maestro output: {error}"
                )));
            }
        }
        Ok(path)
    }

    /// An empty directory for one run's Maestro artifacts, so nothing an
    /// earlier run saved can be read back as this run's.
    fn fresh_test_output_dir(&self) -> E2eResult<PathBuf> {
        let dir = self.directory.path().join("test-output");
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(E2eError::device(format!(
                    "Failed to clear stale Maestro artifacts: {error}"
                )));
            }
        }
        std::fs::create_dir(&dir).map_err(|error| {
            E2eError::device(format!("Failed to create Maestro artifacts dir: {error}"))
        })?;
        Ok(dir)
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
    /// Per-device output workspace.
    workspace: MaestroWorkspace,
    /// The Maestro CLI to run.
    program: PathBuf,
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
            MaestroPlatform::Ios.app_id(),
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
            MaestroPlatform::Android.app_id(),
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
        let workspace = MaestroWorkspace::new()?;

        Ok(Self {
            name: name.into(),
            platform,
            device_name: device_name.into(),
            app_id: app_id.into(),
            relay_url: relay_url.into(),
            flows_dir,
            network_config: NetworkConfig::default(),
            workspace,
            program: PathBuf::from("maestro"),
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
            _ => Err(E2eError::DeviceNotSupported(format!(
                "Maestro CLI not found. {MAESTRO_INSTALL_HELP}"
            ))),
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
        self.run_flow_saving_to(flow_name, env_vars, None).await
    }

    async fn run_flow_saving_to(
        &self,
        flow_name: &str,
        env_vars: &[(&str, &str)],
        test_output_dir: Option<&Path>,
    ) -> E2eResult<String> {
        let flow_path = self.flows_dir.join(format!("{}.yaml", flow_name));

        if !flow_path.exists() {
            return Err(E2eError::device(format!(
                "Maestro flow not found: {}. Create it at: {}",
                flow_name,
                flow_path.display()
            )));
        }

        let mut cmd = Command::new(&self.program);
        cmd.arg("test")
            .arg("--platform")
            .arg(self.platform_flag())
            .arg("--device")
            .arg(&self.device_name);

        // `-e`, not the process environment. Maestro resolves `${VAR}` in a
        // flow from parameters passed with `-e` only; an exported variable
        // of the same name is invisible to it, and an unresolved `${VAR}`
        // is not an error — it interpolates to the literal string
        // "undefined" and gets typed into the app.
        //
        // Passing these as `cmd.env` therefore did nothing for years: every
        // harness-driven flow ran with every variable unset. A device was
        // found carrying an identity named `undefined`, which is this
        // (`backlog/2026-08-09-maestro-flows-accept-unset-inputs.md`).
        //
        // Kept in the environment as well because Maestro reads
        // MAESTRO_APP_ID from there itself.
        cmd.arg("-e")
            .arg(format!("VAUCHI_RELAY_URL={}", self.relay_url));
        for (key, value) in env_vars {
            cmd.arg("-e").arg(format!("{key}={value}"));
        }
        if let Some(dir) = test_output_dir {
            cmd.arg("--test-output-dir").arg(dir);
        }

        cmd.arg(&flow_path)
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
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(E2eError::device(format!(
                "Maestro flow '{}' failed:\nstdout:\n{}\nstderr:\n{}",
                flow_name, stdout, stderr
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn run_output_flow(
        &self,
        flow_name: &str,
        env_vars: &[(&str, &str)],
        output: MaestroOutput,
    ) -> E2eResult<String> {
        let output_path = self.workspace.prepare_output(output)?;
        let output_path = output_path.to_string_lossy().into_owned();
        let mut flow_env = env_vars.to_vec();
        flow_env.push(("MAESTRO_OUTPUT_PATH", output_path.as_str()));
        self.run_flow(flow_name, &flow_env).await?;
        std::fs::read_to_string(self.workspace.output_path(output))
            .map_err(|e| E2eError::device(format!("Failed to read fresh Maestro output: {e}")))
    }

    /// The payload of the QR the flow saved as `screenshot`.
    /// Runs `flow_name` with its artifacts in a fresh directory, returned
    /// for reading back what the flow left there.
    async fn run_flow_with_artifacts(
        &self,
        flow_name: &str,
        env_vars: &[(&str, &str)],
    ) -> E2eResult<PathBuf> {
        let output_dir = self.workspace.fresh_test_output_dir()?;
        self.run_flow_saving_to(flow_name, env_vars, Some(&output_dir))
            .await?;
        Ok(output_dir)
    }

    async fn run_card_edit(&self, flow_name: &str, env_vars: &[(&str, &str)]) -> E2eResult<()> {
        if self.platform == MaestroPlatform::Ios {
            return Err(E2eError::DeviceNotSupported(
                "Maestro device: the iOS card-edit flows are not written yet".into(),
            ));
        }
        self.run_flow_with_artifacts(flow_name, env_vars).await?;
        Ok(())
    }

    fn ios_device_join_unsupported() -> E2eError {
        E2eError::DeviceNotSupported(
            "Maestro device: the iOS device-link joining flows are not written yet".into(),
        )
    }

    async fn run_qr_flow(&self, flow_name: &str, screenshot: &str) -> E2eResult<String> {
        let output_dir = self.run_flow_with_artifacts(flow_name, &[]).await?;
        let path = maestro_artifacts::screenshot_in(&output_dir, screenshot)?;
        let png = std::fs::read(&path)
            .map_err(|e| E2eError::device(format!("Failed to read Maestro screenshot: {e}")))?;
        maestro_artifacts::qr_payload_in(&png)
    }

    fn parse_contacts(content: &str) -> Vec<Contact> {
        content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|name| Contact {
                name: name.trim().to_string(),
                id: None,
                verified: false,
            })
            .collect()
    }

    fn parse_card(content: &str) -> E2eResult<ContactCard> {
        serde_json::from_str(content)
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

    async fn create_identity(&self, name: &str) -> E2eResult<()> {
        self.run_flow("create_identity", &[("NAME", name)]).await?;
        Ok(())
    }

    async fn has_identity(&self) -> bool {
        self.run_output_flow(
            "get_card",
            &[("CONTACT_NAME", "Your Card")],
            MaestroOutput::Card,
        )
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

    async fn generate_qr(&self) -> E2eResult<String> {
        match self.platform {
            MaestroPlatform::Android => {
                self.run_qr_flow("generate_qr", EXCHANGE_QR_SCREENSHOT)
                    .await
            }
            MaestroPlatform::Ios => Err(E2eError::DeviceNotSupported(
                "Maestro device: the iOS simulator exchanges by link, not QR (Glance needs Bluetooth)"
                    .into(),
            )),
        }
    }

    async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()> {
        self.run_flow("complete_exchange", &[("QR_DATA", qr_data)])
            .await?;
        Ok(())
    }

    async fn start_device_link(&self) -> E2eResult<String> {
        match self.platform {
            MaestroPlatform::Android => {
                self.run_qr_flow("link_device", DEVICE_LINK_QR_SCREENSHOT)
                    .await
            }
            MaestroPlatform::Ios => Err(E2eError::DeviceNotSupported(
                "Maestro device: the iOS link_device flow does not save the device-link QR yet"
                    .into(),
            )),
        }
    }

    /// Fresh install: the invitation arrives as the deep link its QR
    /// encodes, which a phone camera would open; returns the confirmation
    /// code the joining device then shows.
    async fn join_identity(&self, qr_data: &str, device_name: &str) -> E2eResult<String> {
        if self.platform == MaestroPlatform::Ios {
            return Err(Self::ios_device_join_unsupported());
        }
        let output_dir = self
            .run_flow_with_artifacts(
                "join_identity",
                &[("INVITATION", qr_data), ("DEVICE_NAME", device_name)],
            )
            .await?;
        maestro_artifacts::flow_output_in(&output_dir, "confirmation_code")
    }

    /// Existing device: approves only if it shows the code the joining
    /// device showed, which is the check a user makes.
    async fn complete_device_link(&self, request_data: &str) -> E2eResult<String> {
        if self.platform == MaestroPlatform::Ios {
            return Err(Self::ios_device_join_unsupported());
        }
        self.run_flow_with_artifacts("complete_device_link", &[("EXPECTED_CODE", request_data)])
            .await?;
        Ok(request_data.to_string())
    }

    async fn finish_device_join(&self, _response_data: &str) -> E2eResult<()> {
        if self.platform == MaestroPlatform::Ios {
            return Err(Self::ios_device_join_unsupported());
        }
        self.run_flow_with_artifacts("finish_device_join", &[])
            .await?;
        Ok(())
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        Err(E2eError::DeviceNotSupported(
            "Maestro device: list_devices requires device-specific flow".into(),
        ))
    }

    async fn sync(&self) -> E2eResult<()> {
        self.run_flow("sync", &[]).await?;
        Ok(())
    }

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        let content = self
            .run_output_flow("list_contacts", &[], MaestroOutput::Contacts)
            .await?;
        Ok(Self::parse_contacts(&content))
    }

    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>> {
        let content = self
            .run_output_flow(
                "get_card",
                &[("CONTACT_NAME", name_or_id)],
                MaestroOutput::Card,
            )
            .await?;
        let card = Self::parse_card(&content)?;
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

    async fn get_card(&self) -> E2eResult<ContactCard> {
        let content = self
            .run_output_flow(
                "get_card",
                &[("CONTACT_NAME", "Your Card")],
                MaestroOutput::Card,
            )
            .await?;
        Self::parse_card(&content)
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

    async fn edit_field(&self, label: &str, value: &str) -> E2eResult<()> {
        self.run_card_edit(
            "edit_field",
            &[("FIELD_LABEL", label), ("FIELD_VALUE", value)],
        )
        .await
    }

    async fn remove_field(&self, label: &str) -> E2eResult<()> {
        self.run_card_edit("remove_field", &[("FIELD_LABEL", label)])
            .await
    }

    async fn edit_name(&self, name: &str) -> E2eResult<()> {
        self.run_card_edit("edit_name", &[("NAME", name)]).await
    }

    async fn set_network(&self, config: NetworkConfig) -> E2eResult<()> {
        let _ = config;
        Ok(())
    }

    fn network_config(&self) -> NetworkConfig {
        self.network_config.clone()
    }

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
#[path = "maestro_tests.rs"]
mod tests;
