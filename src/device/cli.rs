//! CLI-based device implementation.
//!
//! Controls the Vauchi CLI as a subprocess to simulate device operations.

use std::path::PathBuf;
use std::process::Output;

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
}

impl CliDevice {
    /// Create a new CLI device with an isolated data directory.
    pub fn new(name: impl Into<String>, relay_url: impl Into<String>) -> E2eResult<Self> {
        let data_dir = TempDir::new().map_err(|e| {
            E2eError::device(format!("Failed to create temp directory: {}", e))
        })?;

        let cli_path = Self::find_cli_binary()?;

        Ok(Self {
            name: name.into(),
            data_dir,
            relay_url: relay_url.into(),
            cli_path,
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
        })
    }

    /// Find the CLI binary in the workspace.
    fn find_cli_binary() -> E2eResult<PathBuf> {
        // Try release binary first
        let release_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/release/vauchi-cli");
        if release_path.exists() {
            return Ok(release_path);
        }

        // Try debug binary
        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/debug/vauchi-cli");
        if debug_path.exists() {
            return Ok(debug_path);
        }

        Err(E2eError::cli_execution(
            "CLI binary not found. Please run `cargo build -p vauchi-cli` first.",
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

        let output = cmd.output().await.map_err(|e| {
            E2eError::cli_execution(format!("Failed to run CLI command: {}", e))
        })?;

        trace!(
            "CLI stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        trace!(
            "CLI stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

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
    fn parse_contacts(output: &str) -> Vec<Contact> {
        let mut contacts = Vec::new();

        for line in output.lines() {
            // Skip header lines and empty lines
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("Name")
                || line.starts_with("─")
                || line.starts_with("=")
                || line.starts_with("No contacts")
            {
                continue;
            }

            // Try to parse contact line
            // Format varies but typically: "Name" or "Name (id...)"
            let name = if let Some(paren_pos) = line.find('(') {
                line[..paren_pos].trim().to_string()
            } else {
                line.to_string()
            };

            if !name.is_empty() {
                contacts.push(Contact {
                    name,
                    id: None, // Could parse from output if needed
                    verified: false,
                });
            }
        }

        contacts
    }

    /// Parse a contact card from CLI output.
    fn parse_card(output: &str) -> E2eResult<ContactCard> {
        let mut name = String::new();
        let mut fields = Vec::new();

        for line in output.lines() {
            let line = line.trim();

            // Look for the name (usually the first significant line after any headers)
            if line.starts_with("📇") || line.contains("Contact Card") {
                // Try to extract name from the line
                if let Some(colon_pos) = line.find(':') {
                    name = line[colon_pos + 1..].trim().to_string();
                }
                continue;
            }

            // Look for field lines with icons
            if line.contains('│') || line.contains('|') {
                // Parse field from table format
                let parts: Vec<&str> = line.split(|c| c == '│' || c == '|')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();

                if parts.len() >= 2 {
                    // Remove emoji prefix if present
                    let label = parts[0]
                        .trim_start_matches(|c: char| !c.is_alphanumeric())
                        .trim();
                    let value = parts[1].trim();

                    if !label.is_empty() && !value.is_empty() {
                        fields.push(CardField {
                            field_type: "custom".to_string(), // Would need better parsing for type
                            label: label.to_string(),
                            value: value.to_string(),
                        });
                    }
                }
            }

            // Also handle non-table format (Label: Value)
            if let Some(colon_pos) = line.find(':') {
                if !line.contains('│') && !line.contains('|') {
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

            // Try to get name from "Your Contact Card" format
            if name.is_empty() && (line.contains("Name") || line.starts_with("Name:")) {
                if let Some(colon_pos) = line.find(':') {
                    name = line[colon_pos + 1..].trim().to_string();
                }
            }
        }

        // If name still empty, use first line as fallback
        if name.is_empty() {
            if let Some(first_line) = output.lines().next() {
                let first_line = first_line.trim();
                if !first_line.is_empty()
                    && !first_line.starts_with("Your")
                    && !first_line.contains('─')
                {
                    name = first_line.to_string();
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
            if line.chars().all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=') {
                return Ok(line.to_string());
            }

            // Also check for data after "Data:" label
            if line.starts_with("Data:") {
                let data = line[5..].trim();
                if !data.is_empty() {
                    return Ok(data.to_string());
                }
            }
        }

        // If no data found in structured format, try to find any long alphanumeric string
        for line in output.lines() {
            let line = line.trim();
            if line.len() >= 50
                && line.chars().all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
            {
                return Ok(line.to_string());
            }
        }

        Err(E2eError::parse_output("Could not find QR data in CLI output"))
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
        self.run_command_success(&["init", name]).await?;
        Ok(())
    }

    async fn has_identity(&self) -> bool {
        // Check if identity file exists
        let identity_path = self.data_dir.path().join("identity.json");
        identity_path.exists()
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
        self.run_command_success(&["exchange", "complete", qr_data]).await?;
        Ok(())
    }

    // === Device Linking ===

    async fn start_device_link(&self) -> E2eResult<String> {
        let output = self.run_command_success(&["device", "link"]).await?;
        Self::extract_qr_data(&output)
    }

    async fn join_identity(&self, qr_data: &str, _device_name: &str) -> E2eResult<String> {
        // The join command needs device name input - we may need to handle this differently
        // For now, run the command and extract request data
        // Note: device_name would be passed via stdin for interactive prompts in the real CLI
        let output = self.run_command_success(&["device", "join", qr_data]).await?;
        Self::extract_qr_data(&output)
    }

    async fn complete_device_link(&self, request_data: &str) -> E2eResult<String> {
        let output = self.run_command_success(&["device", "complete", request_data]).await?;
        Self::extract_qr_data(&output)
    }

    async fn finish_device_join(&self, response_data: &str) -> E2eResult<()> {
        self.run_command_success(&["device", "finish", response_data]).await?;
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
            // Parse the contact details
            Ok(Some(Contact {
                name: name_or_id.to_string(),
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
        self.run_command_success(&["card", "add", field_type, label, value]).await?;
        Ok(())
    }

    async fn edit_field(&self, label: &str, value: &str) -> E2eResult<()> {
        self.run_command_success(&["card", "edit", label, value]).await?;
        Ok(())
    }

    async fn remove_field(&self, label: &str) -> E2eResult<()> {
        self.run_command_success(&["card", "remove", label]).await?;
        Ok(())
    }

    async fn edit_name(&self, name: &str) -> E2eResult<()> {
        self.run_command_success(&["card", "edit-name", name]).await?;
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
Name          Status
─────────────────────
Alice Smith   Active
Bob Jones     Active
"#;
        let contacts = CliDevice::parse_contacts(output);
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].name, "Alice Smith   Active");
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
        assert_eq!(qr, "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+/=");
    }
}
