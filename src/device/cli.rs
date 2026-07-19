// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! CLI-based device implementation.
//!
//! Controls the Vauchi CLI as a subprocess to simulate device operations.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Output;
use std::sync::Mutex;

use async_trait::async_trait;
use tempfile::TempDir;
use tokio::process::Command;
use tracing::{debug, trace};

use super::{CardField, Contact, ContactCard, Device, DeviceType};
use crate::error::{E2eError, E2eResult};

const ALLOW_DIRECT_ENV: &str = "VAUCHI_ALLOW_DIRECT";

fn configure_command_environment(command: &mut Command, extra_env: &HashMap<String, String>) {
    for (key, value) in extra_env {
        command.env(key, value);
    }

    // The E2E harness must remain fail closed even when the parent shell or a
    // caller tries to inject the retired development escape hatch.
    command.env_remove(ALLOW_DIRECT_ENV);
}

/// Raw JSON representation of `vauchi card show --raw`.
#[derive(Debug, serde::Deserialize)]
struct RawCard {
    display_name: String,
    fields: Vec<RawCardField>,
}

#[derive(Debug, serde::Deserialize)]
struct RawCardField {
    field_type: String,
    label: String,
    value: String,
}

#[derive(Debug, serde::Deserialize)]
struct RawContact {
    card: RawCard,
}

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
    /// Extra env vars to set on every spawned `vauchi` subprocess.
    /// Used by the orchestrator to inject test-only overrides like
    /// `VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX` (F13 step 5).
    extra_env: HashMap<String, String>,
    /// Budget for a single CLI invocation. Parsed once at construction
    /// from `VAUCHI_E2E_CLI_TIMEOUT_SECS`, default 180s: the previous
    /// hardcoded 60s was fitted to median quiet-runner runtime and flaked
    /// under 2-4x runner contention (problem
    /// 2026-05-04-e2e-smoke-cli-timeout-flake, option A). Kept above the
    /// relay's 60s idle cutoff so the two values are not correlated
    /// during debugging.
    command_timeout: std::time::Duration,
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
            extra_env: HashMap::new(),
            command_timeout: Self::command_timeout_from_env(),
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
            extra_env: HashMap::new(),
            command_timeout: Self::command_timeout_from_env(),
        })
    }

    /// Resolve the per-command budget from `VAUCHI_E2E_CLI_TIMEOUT_SECS`.
    fn command_timeout_from_env() -> std::time::Duration {
        Self::resolve_command_timeout(std::env::var("VAUCHI_E2E_CLI_TIMEOUT_SECS").ok().as_deref())
    }

    /// Parse a timeout override in seconds; anything missing or
    /// unparsable falls back to the 180s contention-resilient default.
    fn resolve_command_timeout(value: Option<&str>) -> std::time::Duration {
        let secs = value
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .unwrap_or(180);
        std::time::Duration::from_secs(secs)
    }

    /// Set extra environment variables to pass to every spawned
    /// `vauchi` subprocess. Replaces any previously-set extras. Used
    /// by the orchestrator to inject test-only overrides like
    /// `VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX`.
    pub fn with_extra_env(mut self, extra_env: HashMap<String, String>) -> Self {
        self.extra_env = extra_env;
        self
    }

    /// Find the CLI binary in the workspace.
    fn find_cli_binary() -> E2eResult<PathBuf> {
        // Try E2E_BIN_DIR first (SHA-cached binaries from build-binaries.sh).
        // CI bakes a release-or-debug binary at this path per repo policy.
        if let Ok(dir) = std::env::var("E2E_BIN_DIR") {
            let path = PathBuf::from(&dir).join("vauchi");
            if path.exists() {
                return Ok(path);
            }
        }

        // Prefer the debug binary for local development runs. Production-like
        // coverage uses the SHA-cached `E2E_BIN_DIR` path above. Both builds
        // receive the local ephemeral OHTTP key through the explicit bundled
        // key override, so neither requires a direct transport mode.
        //
        // The cli/ crate is its own Cargo workspace, so a bare `cargo
        // build` in cli/ lands at `cli/target/debug/vauchi`. The
        // workspace-root `target/debug/vauchi` is a historical-residue
        // location with no current producer — check it after the
        // cli-local path so a stale artifact there can't shadow a fresh
        // cli build.
        let cli_local_debug =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cli/target/debug/vauchi");
        if cli_local_debug.exists() {
            return Ok(cli_local_debug);
        }

        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/vauchi");
        if debug_path.exists() {
            return Ok(debug_path);
        }

        // Release fallback retained for tests that inject the ephemeral local
        // OHTTP key through `VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX`.
        let release_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/release/vauchi");
        if release_path.exists() {
            return Ok(release_path);
        }

        Err(E2eError::cli_execution(
            "CLI binary not found. Run `just build cli` (debug) first — \
             the orchestrator supplies the local OHTTP key to either build.",
        ))
    }

    /// Run a CLI command and return the output.
    async fn run_command(&self, args: &[&str]) -> E2eResult<Output> {
        let mut cmd = Command::new(&self.cli_path);
        cmd.arg("--data-dir")
            .arg(self.data_dir.path())
            .arg("--relay")
            .arg(&self.relay_url)
            .args(args)
            .stdin(std::process::Stdio::null())
            // Reap the child when the timeout below fires — otherwise the
            // timed-out `vauchi sync` keeps running, holds its data-dir
            // lock, and compounds runner contention for the rest of the
            // job (problems/2026-07-19-e2e-multi-device-serialization-hang).
            // Guarded by scripts/check-test-timeouts.sh — do not remove.
            .kill_on_drop(true);

        configure_command_environment(&mut cmd, &self.extra_env);

        debug!(
            "Running CLI command: {} --data-dir {} --relay {} {}",
            self.cli_path.display(),
            self.data_dir.path().display(),
            self.relay_url,
            args.join(" ")
        );

        let cmd_desc = format!("vauchi {}", args.join(" "));
        let budget = self.command_timeout;
        let output = tokio::time::timeout(budget, cmd.output())
            .await
            .map_err(|_| {
                E2eError::timeout(format!(
                    "CLI command timed out after {}s: {cmd_desc}",
                    budget.as_secs()
                ))
            })?
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
                || line.starts_with("Missing:")
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

    fn parse_devices(output: &str) -> Vec<String> {
        output
            .lines()
            .filter_map(|line| {
                let (ordinal, device) = line.trim().split_once(". ")?;
                ordinal.parse::<usize>().ok()?;
                let name = device.split_once(" [").map_or(device, |(name, _)| name);
                (!name.is_empty()).then(|| name.to_string())
            })
            .collect()
    }

    fn parse_labels(output: &str) -> Vec<String> {
        let mut labels = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("Visibility")
                || line.starts_with("Label")
                || line.starts_with("No labels")
                || line.starts_with("Contacts:")
                || line.starts_with("Missing:")
                || line.starts_with("ℹ")
                || line.starts_with('─')
                || line.starts_with('╭')
                || line.starts_with('├')
                || line.starts_with('╰')
            {
                continue;
            }

            if line.starts_with('│') {
                let parts: Vec<&str> = line
                    .split('│')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if parts.len() >= 2 && parts[0].parse::<usize>().is_ok() {
                    labels.push(parts[1].to_string());
                }
            } else {
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
        labels
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
    #[allow(dead_code)] // kept as fallback; `get_card()` uses `--raw` JSON
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
                            | "envelope"
                            | "phone"
                            | "📱"
                            | "web"
                            | "🌐"
                            | "globe"
                            | "home"
                            | "🏠"
                            | "mappin"
                            | "social"
                            | "👤"
                            | "note"
                            | "📝"
                            | "tag"
                            | "cake"
                            | "🎂"
                    );

                if is_icon_format {
                    let icon = parts[0];
                    let field_type = match icon {
                        "mail" | "📧" | "envelope" => "email",
                        "phone" | "📱" => "phone",
                        "web" | "🌐" | "globe" => "website",
                        "home" | "🏠" | "mappin" => "address",
                        "social" | "👤" => "social",
                        "note" | "📝" | "tag" => "custom",
                        "cake" | "🎂" => "birthday",
                        _ => "custom",
                    };

                    // The CLI renders card fields as:
                    //   "  {:6} {:12} {}"
                    // icon is padded to a minimum width of 6, label to 12,
                    // then a single space, then the (possibly multi-word)
                    // value. Split on that boundary instead of treating the
                    // last whitespace-separated token as the value.
                    let after_icon = line
                        .trim_start()
                        .strip_prefix(icon)
                        .unwrap_or(line)
                        .trim_start();

                    // Label column is at least 12 chars (left-aligned, space-padded).
                    // The separator space sits immediately after the 12-char column.
                    const LABEL_WIDTH: usize = 12;
                    if after_icon.len() > LABEL_WIDTH {
                        let label_area = &after_icon[..LABEL_WIDTH];
                        let label = label_area.trim_end();
                        let value = after_icon[LABEL_WIDTH..].trim_start();

                        if !label.is_empty() && !value.is_empty() {
                            fields.push(CardField {
                                field_type: field_type.to_string(),
                                label: label.to_string(),
                                value: value.to_string(),
                            });
                        }
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

    /// Parse a contact card from the `--raw` JSON output.
    ///
    /// `--raw` is the preferred parser entry point: it is independent of
    /// icon tokens, column widths, and terminal styling, so CLI display
    /// changes cannot silently break field-level assertions.
    fn parse_card_raw(output: &str) -> E2eResult<ContactCard> {
        let raw: RawCard = serde_json::from_str(output).map_err(|e| {
            E2eError::parse_output(format!("Failed to parse 'card show --raw' JSON: {e}"))
        })?;

        Ok(Self::contact_card_from_raw(raw))
    }

    fn parse_contact_card_raw(output: &str) -> E2eResult<ContactCard> {
        let raw: RawContact = serde_json::from_str(output).map_err(|e| {
            E2eError::parse_output(format!("Failed to parse 'contacts show --raw' JSON: {e}"))
        })?;

        Ok(Self::contact_card_from_raw(raw.card))
    }

    fn contact_card_from_raw(raw: RawCard) -> ContactCard {
        let fields = raw
            .fields
            .into_iter()
            .map(|f| CardField {
                field_type: f.field_type.to_lowercase(),
                label: f.label,
                value: f.value,
            })
            .collect();

        ContactCard {
            name: raw.display_name,
            fields,
        }
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

    async fn generate_qr(&self) -> E2eResult<String> {
        let output = self.run_command_success(&["exchange", "start"]).await?;
        Self::extract_qr_data(&output)
    }

    async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()> {
        self.run_command_success(&["exchange", "complete", qr_data])
            .await?;
        Ok(())
    }

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
            .run_command_success(&["device", "complete", "--yes", request_data])
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
        Ok(Self::parse_devices(&output))
    }

    async fn sync(&self) -> E2eResult<()> {
        // Retry on relay rate-limit (429) with exponential backoff.
        // The test relay enforces per-client token-bucket rate limiting;
        // multi-device sync tests can exhaust the bucket with rapid sequential
        // requests.  Retrying here keeps the test resilient without masking
        // real failures (only "Rate limited" errors are retried).
        const MAX_RETRIES: u32 = 3;
        for attempt in 0..=MAX_RETRIES {
            let output = self.run_command(&["sync"]).await?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Rate limited") && attempt < MAX_RETRIES {
                // Parse "retry after Ns" from stderr, fall back to 10s.
                let retry_after = stderr
                    .find("retry after ")
                    .and_then(|pos| {
                        let rest = &stderr[pos + 12..];
                        rest.split('s').next()?.trim().parse::<u64>().ok()
                    })
                    .unwrap_or(10);
                let wait_secs = retry_after + (attempt as u64 * 2);
                debug!(
                    "Sync rate-limited, retrying in {wait_secs}s (attempt {}/{})",
                    attempt + 1,
                    MAX_RETRIES
                );
                tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                continue;
            }
            return Err(E2eError::CliCommand {
                command: "vauchi sync".to_string(),
                stderr: stderr.to_string(),
            });
        }
        unreachable!("loop runs MAX_RETRIES+1 times and always returns")
    }

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

    async fn get_contact_card(&self, name_or_id: &str) -> E2eResult<Option<ContactCard>> {
        let output = self
            .run_command(&["--raw", "contacts", "show", name_or_id])
            .await?;
        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim_start().starts_with('{') {
            return Ok(None);
        }

        Self::parse_contact_card_raw(&stdout).map(Some)
    }

    async fn get_card(&self) -> E2eResult<ContactCard> {
        let output = self.run_command_success(&["card", "show", "--raw"]).await?;
        Self::parse_card_raw(&output)
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
        Ok(Self::parse_labels(&output))
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

    async fn verify_contact(&self, contact: &str) -> E2eResult<()> {
        self.run_command_success(&["contacts", "verify", contact])
            .await?;
        Ok(())
    }

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

// INLINE_TEST_REQUIRED: tests exercise private CliDevice parsing helpers
// (parse_card, parse_card_raw, extract_qr_data). Keeping them in a child
// module lets them access private APIs while keeping the parent file under
// the size limit.
#[cfg(test)]
mod tests;
