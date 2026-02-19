// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! CLI-based device implementation.
//!
//! Controls the Vauchi CLI as a subprocess to simulate device operations.

use std::path::PathBuf;
use std::process::Output;
use std::sync::Mutex;

use async_trait::async_trait;
use tempfile::TempDir;
use tokio::process::Command;
use tracing::{debug, trace};

use super::{CardField, Contact, ContactCard, Device, DeviceType};
use crate::error::{E2eError, E2eResult};

/// A device controlled via the CLI.
pub struct CliDevice {
    /// Device name/identifier.
    name: String,
    /// Temporary data directory for this device.
    data_dir: TempDir,
    /// Relay URL to connect to.
    relay_url: String,
    /// Path to the CLI binary.
    cli_path: PathBuf,
    /// Public ID captured from init output.
    public_id: Mutex<Option<String>>,
}

impl CliDevice {
    /// Create a new CLI device with an isolated data directory.
    pub fn new(name: impl Into<String>, relay_url: impl Into<String>) -> E2eResult<Self> {
        let data_dir = TempDir::new()
            .map_err(|e| E2eError::device(format!("Failed to create temp directory: {}", e)))?;

        let cli_path = Self::find_cli_binary()?;

        Ok(Self {
            name: name.into(),
            data_dir,
            relay_url: relay_url.into(),
            cli_path,
            public_id: Mutex::new(None),
        })
    }

    /// Create a new CLI device with a specific data directory path.
    pub fn with_data_dir(
        name: impl Into<String>,
        relay_url: impl Into<String>,
        data_dir: TempDir,
    ) -> E2eResult<Self> {
        let cli_path = Self::find_cli_binary()?;

        Ok(Self {
            name: name.into(),
            data_dir,
            relay_url: relay_url.into(),
            cli_path,
            public_id: Mutex::new(None),
        })
    }

    /// Find the CLI binary in the workspace.
    fn find_cli_binary() -> E2eResult<PathBuf> {
        // Try release binary first
        let release_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/release/vauchi");
        if release_path.exists() {
            return Ok(release_path);
        }

        // Try debug binary
        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/vauchi");
        if debug_path.exists() {
            return Ok(debug_path);
        }

        Err(E2eError::cli_execution(
            "CLI binary not found. Please run `just build-cli` first.",
        ))
    }

    /// Get the data directory path.
    pub fn data_dir_path(&self) -> &std::path::Path {
        self.data_dir.path()
    }

    /// Run a CLI command and return the output.
    async fn run_command(&self, args: &[&str]) -> E2eResult<Output> {
        let mut cmd = Command::new(&self.cli_path);
        cmd.arg("--data-dir")
            .arg(self.data_dir.path())
            .arg("--relay")
            .arg(&self.relay_url)
            .args(args);

        debug!(
            "Running CLI command: {} --data-dir {} --relay {} {}",
            self.cli_path.display(),
            self.data_dir.path().display(),
            self.relay_url,
            args.join(" ")
        );

        let output = cmd
            .output()
            .await
            .map_err(|e| E2eError::cli_execution(format!("Failed to run CLI command: {}", e)))?;

        trace!("CLI stdout: {}", String::from_utf8_lossy(&output.stdout));
        trace!("CLI stderr: {}", String::from_utf8_lossy(&output.stderr));

        Ok(output)
    }

    /// Run a CLI command and expect success.
    async fn run_command_success(&self, args: &[&str]) -> E2eResult<String> {
        let output = self.run_command(args).await?;

        if !output.status.success() {
            return Err(E2eError::CliCommand {
                command: format!("vauchi {}", args.join(" ")),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Parse contacts from CLI output.
    ///
    /// Handles the tabled output format:
    /// ```text
    /// Contacts (1):
    ///
    /// ╭───┬──────┬─────────────┬──────────────╮
    /// │ # │ Name │ ID          │ Status       │
    /// ├───┼──────┼─────────────┼──────────────┤
    /// │ 1 │ Bob  │ bcdbedd4... │ not verified │
    /// ╰───┴──────┴─────────────┴──────────────╯
    /// ```
    fn parse_contacts(output: &str) -> Vec<Contact> {
        let mut contacts = Vec::new();

        for line in output.lines() {
            let line = line.trim();

            // Skip empty lines, headers, decorations, and CLI hints
            if line.is_empty()
                || line.starts_with("Contacts")
                || line.starts_with("No contacts")
                || line.starts_with("ℹ")
                || line.starts_with("vauchi")
                // Skip Unicode box-drawing borders
                || line.starts_with('╭')
                || line.starts_with('├')
                || line.starts_with('╰')
                || line.starts_with('─')
                || line.starts_with('=')
            {
                continue;
            }

            // Parse table row: │ # │ Name │ ID │ Status │
            if line.starts_with('│') {
                let parts: Vec<&str> = line
                    .split('│')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();

                // Expected: [#, Name, ID, Status]
                // Skip header row (where first column is "#" or "Name")
                if parts.len() >= 2 {
                    let first = parts[0];
                    // Skip if first column is a header
                    if first == "#" || first == "Name" {
                        continue;
                    }
                    // Data row: first is index number, second is name
                    if first.parse::<usize>().is_ok() && parts.len() >= 2 {
                        let name = parts[1].to_string();
                        let id = parts.get(2).map(|s| s.trim_end_matches("...").to_string());
                        let verified = parts.get(3).map(|s| s.contains('✓')).unwrap_or(false);

                        if !name.is_empty() {
                            contacts.push(Contact { name, id, verified });
                        }
                    }
                }
            } else {
                // Fallback for plain text format: "Name" or "Name (id...)"
                let name = if let Some(paren_pos) = line.find('(') {
                    line[..paren_pos].trim().to_string()
                } else {
                    line.to_string()
                };

                if !name.is_empty() && !name.starts_with("Name") {
                    contacts.push(Contact {
                        name,
                        id: None,
                        verified: false,
                    });
                }
            }
        }

        contacts
    }

    /// Parse a contact card from CLI output.
    ///
    /// The card output format is:
    /// ```text
    /// ──────────────────────────────────────────────────
    ///   Name
    /// ──────────────────────────────────────────────────
    ///   icon   Label        Value
    /// ──────────────────────────────────────────────────
    /// ```
    fn parse_card(output: &str) -> E2eResult<ContactCard> {
        let mut name = String::new();
        let mut fields = Vec::new();
        let mut in_header = true;

        for line in output.lines() {
            let line = line.trim();

            // Skip separator lines
            if line.starts_with('─') || line.is_empty() {
                // After first separator, we're past the header
                if line.starts_with('─') && !name.is_empty() {
                    in_header = false;
                }
                continue;
            }

            // First non-separator line is the name
            if name.is_empty() && !line.starts_with('─') {
                name = line.to_string();
                continue;
            }

            // Skip "(no fields)" indicator
            if line.contains("(no fields)") {
                continue;
            }

            // Parse field lines — three formats, mutually exclusive to avoid duplicates.
            if line.contains('│') || line.contains('|') {
                // Table format (│-separated)
                let parts: Vec<&str> = line
                    .split(['│', '|'])
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();

                if parts.len() >= 2 {
                    let label = parts[0]
                        .trim_start_matches(|c: char| !c.is_alphanumeric())
                        .trim();
                    let value = parts[1].trim();

                    if !label.is_empty() && !value.is_empty() {
                        fields.push(CardField {
                            field_type: "custom".to_string(),
                            label: label.to_string(),
                            value: value.to_string(),
                        });
                    }
                }
            } else if !in_header {
                // Try icon-based column format first
                // Format: "  mail   Work Email   alice@work.com"
                let parts: Vec<&str> = line.split_whitespace().collect();
                let is_icon_format = parts.len() >= 3
                    && matches!(
                        parts[0],
                        "mail"
                            | "📧"
                            | "phone"
                            | "📱"
                            | "web"
                            | "🌐"
                            | "home"
                            | "🏠"
                            | "social"
                            | "👤"
                    );

                if is_icon_format {
                    let icon = parts[0];
                    let field_type = match icon {
                        "mail" | "📧" => "email",
                        "phone" | "📱" => "phone",
                        "web" | "🌐" => "website",
                        "home" | "🏠" => "address",
                        "social" | "👤" => "social",
                        _ => "custom",
                    };

                    let after_icon = line
                        .trim_start()
                        .strip_prefix(icon)
                        .unwrap_or(line)
                        .trim_start();

                    let last_part = parts.last().unwrap();
                    let label = after_icon
                        .strip_suffix(last_part)
                        .unwrap_or(after_icon)
                        .trim();

                    if !label.is_empty() {
                        fields.push(CardField {
                            field_type: field_type.to_string(),
                            label: label.to_string(),
                            value: last_part.to_string(),
                        });
                    }
                } else if let Some(colon_pos) = line.find(':') {
                    // Colon-separated format (Label: Value)
                    let label = line[..colon_pos]
                        .trim_start_matches(|c: char| !c.is_alphanumeric())
                        .trim();
                    let value = line[colon_pos + 1..].trim();

                    if !label.is_empty() && !value.is_empty() && label != "Contact Card" {
                        fields.push(CardField {
                            field_type: "custom".to_string(),
                            label: label.to_string(),
                            value: value.to_string(),
                        });
                    }
                }
            }
        }

        Ok(ContactCard { name, fields })
    }

    /// Extract QR data from CLI output.
    fn extract_qr_data(output: &str) -> E2eResult<String> {
        // Look for lines that contain base64-like data (long string without spaces)
        for line in output.lines() {
            let line = line.trim();

            // Skip empty lines, ASCII art, and labels
            if line.is_empty()
                || line.contains("█")
                || line.contains("▀")
                || line.contains("▄")
                || line.starts_with("QR")
                || line.starts_with("Scan")
                || line.starts_with("Or")
                || line.starts_with("Data:")
                || line.len() < 20
            {
                continue;
            }

            // Check if it looks like base64 data
            if line
                .chars()
                .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
            {
                return Ok(line.to_string());
            }

            // Also check for data after "Data:" label
            if let Some(data) = line.strip_prefix("Data:") {
                let data = data.trim();
                if !data.is_empty() {
                    return Ok(data.to_string());
                }
            }
        }

        // If no data found in structured format, try to find any long alphanumeric string
        for line in output.lines() {
            let line = line.trim();
            if line.len() >= 50
                && line
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
            {
                return Ok(line.to_string());
            }
        }

        Err(E2eError::parse_output(
            "Could not find QR data in CLI output",
        ))
    }
}

#[async_trait]
impl Device for CliDevice {
    fn device_type(&self) -> DeviceType {
        DeviceType::Cli
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn relay_url(&self) -> &str {
        &self.relay_url
    }

    // === Identity Management ===

    async fn create_identity(&self, name: &str) -> E2eResult<()> {
        let output = self.run_command_success(&["init", name]).await?;
        // Capture public ID from init output ("  Public ID: <hex>")
        for line in output.lines() {
            if let Some(pk) = line.trim().strip_prefix("Public ID: ") {
                *self.public_id.lock().unwrap() = Some(pk.trim().to_string());
                break;
            }
        }
        Ok(())
    }

    async fn get_public_id(&self) -> E2eResult<String> {
        self.public_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| E2eError::device("Public ID not available — call create_identity first"))
    }

    async fn has_identity(&self) -> bool {
        // Use CLI status command instead of probing filesystem
        self.run_command(&["card", "show"])
            .await
            .is_ok_and(|o| o.status.success())
    }

    async fn export_identity(&self, path: &str) -> E2eResult<()> {
        self.run_command_success(&["export", path]).await?;
        Ok(())
    }

    async fn import_identity(&self, path: &str) -> E2eResult<()> {
        self.run_command_success(&["import", path]).await?;
        Ok(())
    }

    // === Exchange ===

    async fn generate_qr(&self) -> E2eResult<String> {
        let output = self.run_command_success(&["exchange", "start"]).await?;
        Self::extract_qr_data(&output)
    }

    async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()> {
        self.run_command_success(&["exchange", "complete", qr_data])
            .await?;
        Ok(())
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        let output = self.run_command_success(&["device", "link"]).await?;
        Self::extract_qr_data(&output)
    }

    async fn join_identity(&self, qr_data: &str, device_name: &str) -> E2eResult<String> {
        let output = self
            .run_command_success(&[
                "device",
                "join",
                qr_data,
                "--device-name",
                device_name,
                "--yes",
            ])
            .await?;
        Self::extract_qr_data(&output)
    }

    async fn complete_device_link(&self, request_data: &str) -> E2eResult<String> {
        let output = self
            .run_command_success(&["device", "complete", request_data])
            .await?;
        Self::extract_qr_data(&output)
    }

    async fn finish_device_join(&self, response_data: &str) -> E2eResult<()> {
        self.run_command_success(&["device", "finish", response_data])
            .await?;
        Ok(())
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        let output = self.run_command_success(&["device", "list"]).await?;

        let mut devices = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            // Parse device list output
            if !line.is_empty()
                && !line.starts_with("Device")
                && !line.starts_with("─")
                && !line.starts_with("=")
            {
                // Extract device name from the line
                if let Some(first_space) = line.find(char::is_whitespace) {
                    devices.push(line[..first_space].to_string());
                } else {
                    devices.push(line.to_string());
                }
            }
        }

        Ok(devices)
    }

    // === Sync ===

    async fn sync(&self) -> E2eResult<()> {
        self.run_command_success(&["sync"]).await?;
        Ok(())
    }

    // === Contacts ===

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        let output = self.run_command_success(&["contacts", "list"]).await?;
        Ok(Self::parse_contacts(&output))
    }

    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>> {
        let output = self.run_command(&["contacts", "show", name_or_id]).await?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse the actual name from the first non-empty, non-separator line
            let parsed_name = stdout
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with('─') && !l.starts_with('='))
                .unwrap_or(name_or_id)
                .to_string();

            Ok(Some(Contact {
                name: parsed_name,
                id: None,
                verified: stdout.contains("✓") || stdout.contains("verified"),
            }))
        } else {
            Ok(None)
        }
    }

    // === Card Management ===

    async fn get_card(&self) -> E2eResult<ContactCard> {
        let output = self.run_command_success(&["card", "show"]).await?;
        Self::parse_card(&output)
    }

    async fn add_field(&self, field_type: &str, label: &str, value: &str) -> E2eResult<()> {
        self.run_command_success(&["card", "add", field_type, label, value])
            .await?;
        Ok(())
    }

    async fn edit_field(&self, label: &str, value: &str) -> E2eResult<()> {
        self.run_command_success(&["card", "edit", label, value])
            .await?;
        Ok(())
    }

    async fn remove_field(&self, label: &str) -> E2eResult<()> {
        self.run_command_success(&["card", "remove", label]).await?;
        Ok(())
    }

    async fn edit_name(&self, name: &str) -> E2eResult<()> {
        self.run_command_success(&["card", "edit-name", name])
            .await?;
        Ok(())
    }

    // === Visibility Labels ===

    async fn create_label(&self, name: &str) -> E2eResult<()> {
        self.run_command_success(&["labels", "create", name])
            .await?;
        Ok(())
    }

    async fn delete_label(&self, name: &str) -> E2eResult<()> {
        self.run_command_success(&["labels", "delete", name])
            .await?;
        Ok(())
    }

    async fn list_labels(&self) -> E2eResult<Vec<String>> {
        let output = self.run_command_success(&["labels", "list"]).await?;
        let mut labels = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            // Skip empty lines, headers, sub-info, and decorations
            if line.is_empty()
                || line.starts_with("Visibility")
                || line.starts_with("Label")
                || line.starts_with("No labels")
                || line.starts_with("Contacts:")
                || line.starts_with("ℹ")
                || line.starts_with('─')
                || line.starts_with('╭')
                || line.starts_with('├')
                || line.starts_with('╰')
            {
                continue;
            }

            if line.starts_with('│') {
                // Table format: │ # │ Name │ ... │
                let parts: Vec<&str> = line
                    .split('│')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if parts.len() >= 2 && parts[0].parse::<usize>().is_ok() {
                    labels.push(parts[1].to_string());
                }
            } else {
                // Plain format: "Friends (48312305)" — extract name before parenthetical ID
                let name = if let Some(paren_pos) = line.find('(') {
                    line[..paren_pos].trim().to_string()
                } else {
                    line.to_string()
                };
                if !name.is_empty() && !name.starts_with("Name") {
                    labels.push(name);
                }
            }
        }
        Ok(labels)
    }

    async fn add_contact_to_label(&self, label: &str, contact: &str) -> E2eResult<()> {
        self.run_command_success(&["labels", "add-contact", label, contact])
            .await?;
        Ok(())
    }

    async fn remove_contact_from_label(&self, label: &str, contact: &str) -> E2eResult<()> {
        self.run_command_success(&["labels", "remove-contact", label, contact])
            .await?;
        Ok(())
    }

    async fn show_field_to_label(&self, label: &str, field: &str) -> E2eResult<()> {
        self.run_command_success(&["labels", "show-field", label, field])
            .await?;
        Ok(())
    }

    async fn hide_field_from_label(&self, label: &str, field: &str) -> E2eResult<()> {
        self.run_command_success(&["labels", "hide-field", label, field])
            .await?;
        Ok(())
    }

    // === Contact Visibility ===

    async fn hide_field_from_contact(&self, contact: &str, field: &str) -> E2eResult<()> {
        self.run_command_success(&["contacts", "hide", contact, field])
            .await?;
        Ok(())
    }

    async fn unhide_field_to_contact(&self, contact: &str, field: &str) -> E2eResult<()> {
        self.run_command_success(&["contacts", "unhide", contact, field])
            .await?;
        Ok(())
    }

    // === Contact Verification ===

    async fn verify_contact(&self, contact: &str) -> E2eResult<()> {
        self.run_command_success(&["contacts", "verify", contact])
            .await?;
        Ok(())
    }

    // === Recovery ===

    async fn create_recovery_claim(&self, old_public_key: &str) -> E2eResult<String> {
        let output = self
            .run_command_success(&["recovery", "claim", old_public_key])
            .await?;
        Self::extract_qr_data(&output)
    }

    async fn vouch_for_recovery(&self, claim_data: &str) -> E2eResult<String> {
        let output = self
            .run_command_success(&["recovery", "vouch", claim_data, "--yes"])
            .await?;
        Self::extract_qr_data(&output)
    }

    async fn add_recovery_voucher(&self, voucher_data: &str) -> E2eResult<()> {
        self.run_command_success(&["recovery", "add-voucher", voucher_data])
            .await?;
        Ok(())
    }

    async fn get_recovery_proof(&self) -> E2eResult<Option<String>> {
        let output = self.run_command(&["recovery", "proof"]).await?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.contains("not complete") || stdout.contains("No recovery") {
                return Ok(None);
            }
            Ok(Self::extract_qr_data(&stdout).ok())
        } else {
            Ok(None)
        }
    }

    // === Backup ===

    async fn export_backup(&self, password: &str) -> E2eResult<String> {
        let backup_path = self.data_dir.path().join("backup.vauchi");
        let path_str = backup_path.to_string_lossy().to_string();
        self.run_command_success(&["export", &path_str, "--password", password])
            .await?;
        Ok(path_str)
    }

    async fn import_backup(&self, path: &str, password: &str) -> E2eResult<()> {
        self.run_command_success(&["import", path, "--password", password])
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_contacts_empty() {
        let output = "No contacts found.\n";
        let contacts = CliDevice::parse_contacts(output);
        assert!(contacts.is_empty());
    }

    #[test]
    fn test_parse_contacts_with_data() {
        let output = r#"
Contacts (2):

╭───┬─────────────┬─────────────┬──────────────╮
│ # │ Name        │ ID          │ Status       │
├───┼─────────────┼─────────────┼──────────────┤
│ 1 │ Alice Smith │ abc123...   │ ✓ verified   │
│ 2 │ Bob Jones   │ def456...   │ not verified │
╰───┴─────────────┴─────────────┴──────────────╯
"#;
        let contacts = CliDevice::parse_contacts(output);
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].name, "Alice Smith");
        assert_eq!(contacts[0].id, Some("abc123".to_string()));
        assert!(contacts[0].verified);
        assert_eq!(contacts[1].name, "Bob Jones");
        assert!(!contacts[1].verified);
    }

    #[test]
    fn test_extract_qr_data() {
        let output = r#"
Your exchange QR code:
█████████████
QR data:
abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+/=
"#;
        let qr = CliDevice::extract_qr_data(output).unwrap();
        assert_eq!(
            qr,
            "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+/="
        );
    }
}
