// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fresh full restore must preserve sharing choices and existing relationships.

use vauchi_e2e_tests::device::{CliDevice, Device};

use crate::ohttp_helpers::spawn_ohttp_stack;

fn device(name: &str, relay: &str, outer: &str, anchor: &[u8; 32]) -> CliDevice {
    let mut device = CliDevice::new(name, relay).unwrap();
    device
        .set_command_env("VAUCHI_OHTTP_RELAY_URL", outer)
        .unwrap();
    device
        .set_command_env("VAUCHI_RELAY_ANCHOR", &hex::encode(anchor))
        .unwrap();
    device
}

async fn sync_pair(alice: &CliDevice, bob: &CliDevice) {
    alice.sync().await.unwrap();
    bob.sync().await.unwrap();
}

// @scenario: backup_format_versioning :: Restored contacts continue exchanging updates
#[tokio::test]
async fn integration_full_backup_restore_preserves_sharing_and_two_way_updates() {
    let (mut relays, mut outer, relay_url, outer_url) = spawn_ohttp_stack().await;
    let anchor = relays.relay(0).unwrap().ohttp_anchor().unwrap();
    let alice = device("Alice", &relay_url, &outer_url, &anchor);
    let bob = device("Bob", &relay_url, &outer_url, &anchor);
    alice.create_identity("Alice").await.unwrap();
    bob.create_identity("Bob").await.unwrap();
    let public_id = alice.get_public_id().await.unwrap();
    let alice_qr = alice.generate_qr().await.unwrap();
    let bob_qr = bob.generate_qr().await.unwrap();
    alice.complete_exchange(&bob_qr).await.unwrap();
    bob.complete_exchange(&alice_qr).await.unwrap();
    alice
        .add_field("email", "SharedEmail", "old@example.test")
        .await
        .unwrap();
    alice
        .add_field("phone", "PrivatePhone", "+12025550164")
        .await
        .unwrap();
    alice.create_label("Work").await.unwrap();
    alice.add_contact_to_label("Work", "Bob").await.unwrap();
    alice
        .show_field_to_label("Work", "SharedEmail")
        .await
        .unwrap();
    alice
        .show_field_to_label("Work", "PrivatePhone")
        .await
        .unwrap();
    alice
        .set_label_bio_override("Work", Some("Work biography"))
        .await
        .unwrap();
    alice
        .hide_field_from_contact("Bob", "PrivatePhone")
        .await
        .unwrap();
    bob.add_field("phone", "BobPhone", "+12025550264")
        .await
        .unwrap();
    bob.unhide_field_to_contact("Alice", "BobPhone")
        .await
        .unwrap();
    for _ in 0..3 {
        sync_pair(&alice, &bob).await;
    }
    let choices = alice.label_state("Work").await.unwrap();
    assert_eq!(choices.visible_fields, ["PrivatePhone", "SharedEmail"]);
    assert_eq!(choices.bio_override.as_deref(), Some("Work biography"));
    let before = alice.get_card().await.unwrap();
    let backup = alice
        .export_full_backup("correct-horse-battery-staple")
        .await
        .unwrap();
    let saved = tempfile::tempdir().unwrap();
    let backup_path = saved.path().join("full.vauchi");
    std::fs::copy(backup, &backup_path).unwrap();
    drop(alice);

    let restored = device("Alice replacement", &relay_url, &outer_url, &anchor);
    let path = backup_path.to_str().unwrap();
    assert!(!restored.has_identity().await);
    let error = restored
        .import_full_backup(path, "wrong-password")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Backup validation failed"),
        "{error}"
    );
    assert!(!restored.has_identity().await);
    restored
        .import_full_backup(path, "correct-horse-battery-staple")
        .await
        .unwrap();

    // Each adapter command launches a fresh CLI process and reopens storage.
    assert_eq!(restored.get_public_id().await.unwrap(), public_id);
    let card = restored.get_card().await.unwrap();
    assert_eq!(card.name, before.name);
    assert_eq!(card.fields.len(), before.fields.len());
    let actual_fields: Vec<_> = card
        .fields
        .iter()
        .map(|f| (&f.field_type, &f.label, &f.value))
        .collect();
    let expected_fields: Vec<_> = before
        .fields
        .iter()
        .map(|f| (&f.field_type, &f.label, &f.value))
        .collect();
    assert_eq!(actual_fields, expected_fields);
    assert_eq!(restored.list_contacts().await.unwrap().len(), 1);
    assert_eq!(restored.label_state("Work").await.unwrap(), choices);
    assert!(
        !restored
            .contact_field_visibility("Bob", "PrivatePhone")
            .await
            .unwrap()
            .visible
    );
    restored
        .edit_field("SharedEmail", "new@example.test")
        .await
        .unwrap();
    bob.edit_field("BobPhone", "+12025550364").await.unwrap();
    let mut arrived = false;
    for _ in 0..6 {
        sync_pair(&restored, &bob).await;
        let from_alice = bob.get_contact_card("Alice").await.unwrap().unwrap();
        let from_bob = restored.get_contact_card("Bob").await.unwrap().unwrap();
        assert!(!from_alice.fields.iter().any(|f| f.label == "PrivatePhone"));
        arrived = from_alice
            .fields
            .iter()
            .any(|f| f.label == "SharedEmail" && f.value == "new@example.test")
            && from_bob
                .fields
                .iter()
                .any(|f| f.label == "BobPhone" && f.value == "+12025550364");
        if arrived {
            break;
        }
    }
    assert!(
        arrived,
        "fresh restore must resume both directions without exchanging again"
    );
    outer.stop().await;
    relays.stop_all().await;
}
