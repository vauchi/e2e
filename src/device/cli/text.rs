// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parsers for the CLI's human-readable output.

use super::CliDevice;
use crate::device::{CardField, ContactCard};
use crate::error::E2eResult;

impl CliDevice {
    pub(super) fn parse_labels(output: &str) -> Vec<String> {
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
    pub(super) fn parse_card(output: &str) -> E2eResult<ContactCard> {
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
}
