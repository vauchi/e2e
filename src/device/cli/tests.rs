// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// INLINE_TEST_REQUIRED: tests depend on private CLI command helpers
use std::collections::HashMap;

use tokio::process::Command;

use super::{CliDevice, configure_command_environment, rate_limit_retry_after};

// @internal
#[test]
fn sync_retry_delay_detects_rate_limit_warning_on_success_stdout() {
    let stdout = b"\xe2\x9c\x93 Connected\n\n\xe2\x84\xb9 Sync complete: No new messages or pending updates\n\
        \xe2\x9a\xa0 Sync error: incoming: Rate limited (retry after 10s)\n";

    assert_eq!(rate_limit_retry_after(stdout, b""), Some(10));
    assert_eq!(
        rate_limit_retry_after(b"", b"Rate limited (retry after 7s)"),
        Some(7)
    );
    assert_eq!(
        rate_limit_retry_after(b"Sync complete: No new messages", b""),
        None
    );
}

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
fn test_parse_contacts_raw_preserves_full_contact_ids() {
    let contacts = CliDevice::parse_contacts_raw(
        r#"[{"id":"aabbccddeeff00112233445566778899","display_name":"Bob","fingerprint_verified":true,"recovery_trusted":false,"card":{"display_name":"Bob","fields":[]}}]"#,
    )
    .expect("raw contacts list should parse");

    assert_eq!(contacts.len(), 1);
    assert_eq!(
        contacts[0].id.as_deref(),
        Some("aabbccddeeff00112233445566778899")
    );
    assert!(contacts[0].verified);
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
fn device_id_parser_selects_the_requested_linked_device() {
    let output = r#"
Linked devices:
  1. Alice_0 [active] (this device)
     ID: 0102030405060708...
  2. Alice_1 [active]
     ID: 1112131415161718...
"#;

    assert_eq!(
        CliDevice::device_id_for_name(output, "Alice_1"),
        Some("1112131415161718".to_string())
    );
    assert_eq!(CliDevice::device_id_for_name(output, "Alice_2"), None);
}

// @internal
#[test]
fn label_parser_ignores_missing_localization_diagnostics() {
    let output = r#"
Missing: cli.labels.list.header
╭───┬─────────┬──────────╮
│ # │ Name    │ Contacts │
├───┼─────────┼──────────┤
│ 1 │ Work    │ 1        │
│ 2 │ Friends │ 1        │
╰───┴─────────┴──────────╯
"#;

    assert_eq!(CliDevice::parse_labels(output), vec!["Work", "Friends"]);
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

// @internal
#[test]
fn command_timeout_resolution_defaults_to_contention_headroom() {
    assert_eq!(
        CliDevice::resolve_command_timeout(None),
        std::time::Duration::from_secs(180),
        "default CLI command budget must clear the relay 60s idle cutoff \
         with p99 runner-contention headroom (problem \
         2026-05-04-e2e-smoke-cli-timeout-flake, option A)"
    );
}

// @internal
#[test]
fn command_timeout_resolution_honours_env_override() {
    assert_eq!(
        CliDevice::resolve_command_timeout(Some("2")),
        std::time::Duration::from_secs(2)
    );
    assert_eq!(
        CliDevice::resolve_command_timeout(Some("not-a-number")),
        std::time::Duration::from_secs(180),
        "unparsable override must fall back to the default, not panic"
    );
}

// @internal
#[tokio::test]
async fn command_timeout_fires_with_command_description() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let sleeper = dir.path().join("sleeper.sh");
    std::fs::write(&sleeper, "#!/bin/sh\nsleep 5\n").expect("write sleeper");
    let mut perms = std::fs::metadata(&sleeper).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&sleeper, perms).expect("chmod sleeper");

    let data_dir = tempfile::tempdir().expect("data dir");
    let device = CliDevice {
        name: "timeout-probe".to_string(),
        data_dir,
        relay_url: "https://127.0.0.1:1".to_string(),
        cli_path: sleeper,
        public_id: std::sync::Mutex::new(None),
        extra_env: HashMap::new(),
        command_timeout: std::time::Duration::from_secs(1),
    };

    let error = device
        .run_command(&["card", "show"])
        .await
        .expect_err("a 5s command must exceed the 1s budget");
    let message = error.to_string();
    assert!(
        message.contains("card show"),
        "timeout error must keep the command description for triage: {message}"
    );
    assert!(
        message.contains("1s"),
        "timeout error must state the configured budget: {message}"
    );
}
