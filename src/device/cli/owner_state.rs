// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parsers for the CLI's owner-private state readouts (`labels show`,
//! `tags list`). Members and fields come back sorted so two devices'
//! readouts compare byte-for-byte.

use crate::device::{ContactFieldVisibility, LabelState, TagState, VisibilitySource};
use crate::error::{E2eError, E2eResult};

const OVERRIDES_HEADER: &str = "Presentation overrides:";

#[derive(PartialEq)]
enum Section {
    Preamble,
    Members,
    VisibleFields,
    Overrides,
}

/// Parse `vauchi labels show <label>`.
pub(super) fn parse_label_state(output: &str) -> E2eResult<LabelState> {
    let mut state = LabelState::default();
    let mut section = Section::Preamble;
    let mut saw_overrides = false;

    for raw in output.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("Label:") {
            state.name = name.trim().to_string();
        } else if line.starts_with("Contacts:") {
            section = Section::Members;
        } else if line.starts_with("Visible fields:") {
            section = Section::VisibleFields;
        } else if line == OVERRIDES_HEADER {
            section = Section::Overrides;
            saw_overrides = true;
        } else if let Some(item) = line.strip_prefix("- ") {
            match section {
                Section::Members => state.members.push(strip_id_suffix(item)),
                Section::VisibleFields => state.visible_fields.push(item.trim().to_string()),
                Section::Preamble | Section::Overrides => {}
            }
        } else if section == Section::Overrides {
            if let Some(value) = line.strip_prefix("Name:") {
                state.name_override = override_value(value);
            } else if let Some(value) = line.strip_prefix("Bio:") {
                state.bio_override = override_value(value);
            } else if let Some(value) = line.strip_prefix("Avatar:") {
                state.avatar_override_bytes = override_value(value)
                    .map(|size| parse_byte_count(&size))
                    .transpose()?;
            }
        }
    }

    if !saw_overrides {
        return Err(E2eError::parse_output(format!(
            "`labels show` output lacks the `{OVERRIDES_HEADER}` block: {output:?}"
        )));
    }
    state.members.sort();
    state.visible_fields.sort();
    Ok(state)
}

/// Parse `vauchi tags list`.
pub(super) fn parse_tags(output: &str) -> Vec<TagState> {
    let mut tags: Vec<TagState> = Vec::new();
    for raw in output.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("Contacts:") || line.starts_with('\u{2139}') {
            continue;
        }
        if let Some(member) = line.strip_prefix("- ") {
            if let Some(tag) = tags.last_mut() {
                tag.members.push(member.trim().to_string());
            }
        } else {
            tags.push(TagState {
                name: strip_id_suffix(line),
                members: Vec::new(),
            });
        }
    }
    for tag in &mut tags {
        tag.members.sort();
    }
    tags
}

/// `Bob (5f3a9c1d)` → `Bob`.
fn strip_id_suffix(item: &str) -> String {
    item.rsplit_once(" (")
        .map_or(item, |(name, _)| name)
        .trim()
        .to_string()
}

fn override_value(value: &str) -> Option<String> {
    let value = value.trim();
    (value != "-").then(|| value.to_string())
}

fn parse_byte_count(value: &str) -> E2eResult<usize> {
    value
        .strip_suffix(" bytes")
        .and_then(|count| count.parse().ok())
        .ok_or_else(|| {
            E2eError::parse_output(format!("unexpected avatar override size: {value:?}"))
        })
}

/// Parse one field's row from `vauchi contacts visibility <contact>`:
/// `<status> <label> [override|inherited]: <value>`, where the status is
/// `✓ visible`, `✗ hidden`, or either with ` (restricted)`.
pub(super) fn parse_contact_field_visibility(
    output: &str,
    field: &str,
) -> E2eResult<ContactFieldVisibility> {
    for line in output.lines() {
        for (marker, source) in [
            (" [override]:", VisibilitySource::Override),
            (" [inherited]:", VisibilitySource::Inherited),
        ] {
            let Some(end) = line.find(marker) else {
                continue;
            };
            let Some(status) = line[..end].strip_suffix(field) else {
                continue;
            };
            let Some(status) = status.strip_suffix(' ') else {
                continue;
            };
            let status = status.trim();
            let visible = if status.starts_with('\u{2713}') {
                true
            } else if status.starts_with('\u{2717}') {
                false
            } else {
                return Err(E2eError::parse_output(format!(
                    "field {field:?} has visibility status {status:?}, not visible or hidden"
                )));
            };
            return Ok(ContactFieldVisibility { visible, source });
        }
    }
    Err(E2eError::parse_output(format!(
        "no visibility row for field {field:?} in: {output:?}"
    )))
}
