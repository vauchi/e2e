// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Device abstraction for E2E testing.
//!
//! Provides a trait-based abstraction over different device types:
//! - CLI: Command-line based control (fully implemented)
//! - TUI: Terminal UI control (stub - requires expectrl)
//! - Tauri: Desktop app control (stub - requires WebdriverIO)
//! - Maestro: Mobile app control (stub - requires Maestro CLI)

mod cli;
mod maestro;
mod tauri;
mod tui;

pub use cli::CliDevice;
pub use maestro::{MaestroDevice, MaestroPlatform};
pub use tauri::TauriDevice;
pub use tui::TuiDevice;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::E2eResult;

/// Network simulation configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    /// Whether internet is available.
    pub internet: bool,

    /// Network mode for simulation.
    pub mode: NetworkMode,

    /// Packet drop rate (0.0 to 1.0) for flaky mode.
    pub drop_rate: f32,

    /// Added latency in milliseconds.
    pub latency_ms: u32,
}

/// Network simulation modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    /// Normal network conditions.
    #[default]
    Normal,

    /// Flaky network with packet drops.
    Flaky,

    /// Complete network outage.
    Offline,
}

/// Types of devices that can be controlled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    /// Command-line interface device.
    Cli,
    /// iOS simulator (via Maestro).
    IosSimulator,
    /// Android emulator (via Maestro).
    AndroidEmulator,
    /// Desktop app (via WebdriverIO/Tauri).
    Desktop,
    /// Terminal UI.
    Tui,
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceType::Cli => write!(f, "CLI"),
            DeviceType::IosSimulator => write!(f, "iOS"),
            DeviceType::AndroidEmulator => write!(f, "Android"),
            DeviceType::Desktop => write!(f, "Desktop"),
            DeviceType::Tui => write!(f, "TUI"),
        }
    }
}

/// A contact in the contact list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    /// Contact's display name.
    pub name: String,
    /// Contact's public ID (if available).
    pub id: Option<String>,
    /// Whether the contact is verified.
    pub verified: bool,
}

/// A field on a contact card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardField {
    /// Field type (email, phone, etc.).
    pub field_type: String,
    /// Field label.
    pub label: String,
    /// Field value.
    pub value: String,
}

/// A contact card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactCard {
    /// Display name.
    pub name: String,
    /// Card fields.
    pub fields: Vec<CardField>,
}

/// Trait for devices that can be controlled in E2E tests.
#[async_trait]
pub trait Device: Send + Sync {
    /// Get the device type.
    fn device_type(&self) -> DeviceType;

    /// Get the device name/identifier.
    fn name(&self) -> &str;

    /// Get the relay URL this device connects to.
    fn relay_url(&self) -> &str;

    // === Identity Management ===

    /// Create a new identity with the given display name.
    async fn create_identity(&self, name: &str) -> E2eResult<()>;

    /// Check if an identity exists.
    async fn has_identity(&self) -> bool;

    /// Export identity to a path.
    async fn export_identity(&self, path: &str) -> E2eResult<()>;

    /// Import identity from a path.
    async fn import_identity(&self, path: &str) -> E2eResult<()>;

    // === Exchange ===

    /// Generate a QR code for exchange (returns the QR data string).
    async fn generate_qr(&self) -> E2eResult<String>;

    /// Complete an exchange using a QR code from another user.
    async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()>;

    // === Device Linking ===

    /// Start the device linking process (returns QR data for new device).
    async fn start_device_link(&self) -> E2eResult<String>;

    /// Join an existing identity using QR from another device (returns request data).
    async fn join_identity(&self, qr_data: &str, device_name: &str) -> E2eResult<String>;

    /// Complete device linking (called on existing device with request from new device).
    async fn complete_device_link(&self, request_data: &str) -> E2eResult<String>;

    /// Finish device join (called on new device with response from existing device).
    async fn finish_device_join(&self, response_data: &str) -> E2eResult<()>;

    /// List linked devices.
    async fn list_devices(&self) -> E2eResult<Vec<String>>;

    // === Sync ===

    /// Sync with the relay server.
    async fn sync(&self) -> E2eResult<()>;

    // === Contacts ===

    /// List all contacts.
    async fn list_contacts(&self) -> E2eResult<Vec<Contact>>;

    /// Get a specific contact by name or ID.
    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>>;

    // === Card Management ===

    /// Get the user's contact card.
    async fn get_card(&self) -> E2eResult<ContactCard>;

    /// Add a field to the contact card.
    async fn add_field(&self, field_type: &str, label: &str, value: &str) -> E2eResult<()>;

    /// Edit a field on the contact card.
    async fn edit_field(&self, label: &str, value: &str) -> E2eResult<()>;

    /// Remove a field from the contact card.
    async fn remove_field(&self, label: &str) -> E2eResult<()>;

    /// Update the display name.
    async fn edit_name(&self, name: &str) -> E2eResult<()>;

    // === Visibility Labels ===

    /// Create a visibility label.
    async fn create_label(&self, _name: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Labels not supported on this device type".to_string(),
        ))
    }

    /// Delete a visibility label.
    async fn delete_label(&self, _name: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Labels not supported on this device type".to_string(),
        ))
    }

    /// List all visibility labels.
    async fn list_labels(&self) -> E2eResult<Vec<String>> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Labels not supported on this device type".to_string(),
        ))
    }

    /// Add a contact to a label.
    async fn add_contact_to_label(&self, _label: &str, _contact: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Labels not supported on this device type".to_string(),
        ))
    }

    /// Remove a contact from a label.
    async fn remove_contact_from_label(&self, _label: &str, _contact: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Labels not supported on this device type".to_string(),
        ))
    }

    /// Show a field to contacts in a label.
    async fn show_field_to_label(&self, _label: &str, _field: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Labels not supported on this device type".to_string(),
        ))
    }

    /// Hide a field from contacts in a label.
    async fn hide_field_from_label(&self, _label: &str, _field: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Labels not supported on this device type".to_string(),
        ))
    }

    // === Contact Visibility ===

    /// Hide a field from a specific contact.
    async fn hide_field_from_contact(&self, _contact: &str, _field: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Contact visibility not supported on this device type".to_string(),
        ))
    }

    /// Show a field to a specific contact.
    async fn unhide_field_to_contact(&self, _contact: &str, _field: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Contact visibility not supported on this device type".to_string(),
        ))
    }

    // === Identity Info ===

    /// Get the public identity ID (hex-encoded signing public key).
    ///
    /// Available after `create_identity` is called. Used for recovery claims.
    async fn get_public_id(&self) -> E2eResult<String> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Public ID not available on this device type".to_string(),
        ))
    }

    // === Contact Verification ===

    /// Mark a contact's fingerprint as verified.
    async fn verify_contact(&self, _contact: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Contact verification not supported on this device type".to_string(),
        ))
    }

    // === Recovery ===

    /// Create a recovery claim (returns claim data).
    async fn create_recovery_claim(&self, _old_public_key: &str) -> E2eResult<String> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Recovery not supported on this device type".to_string(),
        ))
    }

    /// Vouch for someone's recovery claim (returns voucher data).
    async fn vouch_for_recovery(&self, _claim_data: &str) -> E2eResult<String> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Recovery not supported on this device type".to_string(),
        ))
    }

    /// Add a voucher to a recovery proof.
    async fn add_recovery_voucher(&self, _voucher_data: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Recovery not supported on this device type".to_string(),
        ))
    }

    /// Get recovery proof (returns proof data if complete).
    async fn get_recovery_proof(&self) -> E2eResult<Option<String>> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Recovery not supported on this device type".to_string(),
        ))
    }

    // === Backup ===

    /// Export identity backup (returns backup file path).
    async fn export_backup(&self, _password: &str) -> E2eResult<String> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Backup not supported on this device type".to_string(),
        ))
    }

    /// Import identity from backup.
    async fn import_backup(&self, _path: &str, _password: &str) -> E2eResult<()> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Backup not supported on this device type".to_string(),
        ))
    }

    // === Network Simulation ===

    /// Set network conditions for this device.
    ///
    /// This is used for testing offline scenarios, flaky networks, etc.
    /// Not all device types support this - defaults to no-op.
    async fn set_network(&self, _config: NetworkConfig) -> E2eResult<()> {
        // Default implementation does nothing
        Ok(())
    }

    /// Get current network configuration.
    fn network_config(&self) -> NetworkConfig {
        NetworkConfig::default()
    }

    // === App Lifecycle (Mobile/Desktop) ===

    /// Background the app (move to background state).
    ///
    /// Only applicable for mobile and desktop devices.
    async fn background_app(&self) -> E2eResult<()> {
        // Default implementation does nothing (CLI doesn't have background state)
        Ok(())
    }

    /// Foreground the app (bring to front).
    ///
    /// Only applicable for mobile and desktop devices.
    async fn foreground_app(&self) -> E2eResult<()> {
        // Default implementation does nothing
        Ok(())
    }

    /// Kill the app process.
    ///
    /// Only applicable for mobile and desktop devices.
    async fn kill_app(&self) -> E2eResult<()> {
        // Default implementation does nothing
        Ok(())
    }

    /// Launch the app.
    ///
    /// Only applicable for mobile and desktop devices.
    async fn launch_app(&self) -> E2eResult<()> {
        // Default implementation does nothing
        Ok(())
    }

    /// Simulate device restart.
    ///
    /// This simulates app cold start after device reboot.
    async fn restart_device(&self) -> E2eResult<()> {
        // Default implementation: kill and launch
        self.kill_app().await?;
        self.launch_app().await?;
        Ok(())
    }

    // === Proximity Verification (Mobile) ===

    /// Start proximity verification (generates audio challenge).
    ///
    /// Only applicable for mobile devices with audio capability.
    async fn start_proximity_verification(&self) -> E2eResult<String> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Proximity verification not supported on this device type".to_string(),
        ))
    }

    /// Verify proximity using audio challenge.
    ///
    /// Only applicable for mobile devices with audio capability.
    async fn verify_proximity(&self, _challenge: &str) -> E2eResult<bool> {
        Err(crate::error::E2eError::DeviceNotSupported(
            "Proximity verification not supported on this device type".to_string(),
        ))
    }

    // === Device Capabilities ===

    /// Check if this device supports network simulation.
    fn supports_network_simulation(&self) -> bool {
        false
    }

    /// Check if this device supports app lifecycle control.
    fn supports_lifecycle_control(&self) -> bool {
        false
    }

    /// Check if this device supports proximity verification.
    fn supports_proximity_verification(&self) -> bool {
        false
    }
}
