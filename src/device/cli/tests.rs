// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// INLINE_TEST_REQUIRED: tests depend on private CLI command helpers
use std::collections::HashMap;

use tokio::process::Command;

use super::{CliDevice, configure_command_environment};

// @internal
#[test]
fn command_environment_removes_direct_transport_escape_hatch() {
    let mut command = Command::new("vauchi");
    let mut extra_env = HashMap::new();
    extra_env.insert("SAFE_TEST_VALUE".to_string(), "present".to_string());
    extra_env.insert("VAUCHI_ALLOW_DIRECT".to_string(), "1".to_string());

    configure_command_environment(&mut command, &extra_env);

    let environment: HashMap<_, _> = command.as_std().get_envs().collect();
    assert_eq!(
        environment.get(std::ffi::OsStr::new("SAFE_TEST_VALUE")),
        Some(&Some(std::ffi::OsStr::new("present")))
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("VAUCHI_ALLOW_DIRECT")),
        Some(&None),
        "E2E subprocesses must remove the direct-transport escape hatch even when inherited or injected"
    );
}

// @internal
#[test]
fn test_parse_contacts_empty() {
    let output = "No contacts found.\n";
    let contacts = CliDevice::parse_contacts(output);
    assert!(contacts.is_empty());
}

// @internal
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

// @internal
#[test]
fn contact_parser_ignores_missing_localization_diagnostics() {
    let output = r#"
Missing: cli.contacts.list.header
╭───┬──────┬─────────────┬──────────────╮
│ # │ Name │ ID          │ Status       │
├───┼──────┼─────────────┼──────────────┤
│ 1 │ Bob  │ def456...   │ not verified │
╰───┴──────┴─────────────┴──────────────╯
"#;

    let contacts = CliDevice::parse_contacts(output);

    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].name, "Bob");
}

// @scenario: release_privacy_multidevice_certification.feature:Every active device can exchange and update
#[test]
fn device_list_parser_returns_only_numbered_device_rows() {
    let output = r#"
ℹ Current device: Alice_0 (index 0)
  Device ID: 0102030405060708

Linked devices:
──────────────────────────────────────────────────
  1. Alice_0 [active] (this device)
     ID: 0102030405060708...
  2. Alice_1 [active]
     ID: 1112131415161718...
  3. Alice_2 [active]
     ID: 2122232425262728...
──────────────────────────────────────────────────
Total: 3
"#;

    assert_eq!(
        CliDevice::parse_devices(output),
        vec!["Alice_0", "Alice_1", "Alice_2"]
    );
}

// @internal
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

// @internal
#[test]
fn test_parse_card_empty() {
    let output = r#"
──────────────────────────────────────────────────
  Alice
──────────────────────────────────────────────────
  (no fields)
──────────────────────────────────────────────────
"#;
    let card = CliDevice::parse_card(output).unwrap();
    assert_eq!(card.name, "Alice");
    assert!(card.fields.is_empty());
}

// @internal
#[test]
fn test_parse_card_formatted_output() {
    // Regression guard for the human-readable parser: it must tolerate
    // current icon tokens and multi-word values. Labels >12 chars overflow
    // CLI's fixed column, so this test uses short labels.
    let output = r#"
──────────────────────────────────────────────────
  Alice
──────────────────────────────────────────────────
  envelope Public      alice@public.com
  phone    Mobile      +15550199
  globe    Web         https://example.com
  mappin   Home        123 Main St
  tag      Note        hello world
──────────────────────────────────────────────────
"#;
    let card = CliDevice::parse_card(output).unwrap();
    assert_eq!(card.name, "Alice");
    assert_eq!(card.fields.len(), 5);

    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "email" && f.value == "alice@public.com")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "phone" && f.value == "+15550199")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "website" && f.value == "https://example.com")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "address" && f.value == "123 Main St")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "custom" && f.value == "hello world")
    );
}

// @internal
#[test]
fn test_parse_card_raw() {
    // `get_card()` uses `card show --raw`; this is the load-bearing parser
    // for field-level assertions. It must handle long labels and multi-word
    // values without being affected by icon/column changes.
    let output = r#"{
  "display_name": "Alice",
  "fields": [
{
  "field_type": "Email",
  "label": "Public Email",
  "value": "alice@public.com"
},
{
  "field_type": "Phone",
  "label": "Private Phone",
  "value": "+15550199"
},
{
  "field_type": "Website",
  "label": "Web",
  "value": "https://example.com"
},
{
  "field_type": "Address",
  "label": "Home",
  "value": "123 Main St"
},
{
  "field_type": "Custom",
  "label": "Note",
  "value": "hello world"
}
  ]
}"#;
    let card = CliDevice::parse_card_raw(output).unwrap();
    assert_eq!(card.name, "Alice");
    assert_eq!(card.fields.len(), 5);

    assert!(card.fields.iter().any(|f| f.field_type == "email"
        && f.label == "Public Email"
        && f.value == "alice@public.com"));
    assert!(
        card.fields.iter().any(|f| f.field_type == "phone"
            && f.label == "Private Phone"
            && f.value == "+15550199")
    );
    assert!(card.fields.iter().any(|f| f.field_type == "website"
        && f.label == "Web"
        && f.value == "https://example.com"));
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "address" && f.label == "Home" && f.value == "123 Main St")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "custom" && f.label == "Note" && f.value == "hello world")
    );
}

// @internal
#[test]
fn test_parse_card_legacy_icons() {
    // Older CLI builds still emit legacy icons; the parser must handle both.
    let output = r#"
──────────────────────────────────────────────────
  Alice
──────────────────────────────────────────────────
  mail   Work Email   alice@work.com
  web    Personal     https://example.org
  home   Home         123 Main St
  note   Memo         remember this
──────────────────────────────────────────────────
"#;
    let card = CliDevice::parse_card(output).unwrap();
    assert_eq!(card.fields.len(), 4);
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "email" && f.value == "alice@work.com")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "website" && f.value == "https://example.org")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "address" && f.value == "123 Main St")
    );
    assert!(
        card.fields
            .iter()
            .any(|f| f.field_type == "custom" && f.value == "remember this")
    );
}
