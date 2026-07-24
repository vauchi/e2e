// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Resistance Features E2E Tests
//!
//! End-to-end validation of duress PIN, emergency broadcast, hidden contacts,
//! and panic shred across the full contact lifecycle.
//!
//! Feature: resistance_features.feature @duress @emergency @hidden @panic

use vauchi_core::{AuthMode, Contact, ContactCard, DuressSettings, SymmetricKey, Vauchi};

// Helper to create a test contact with a unique PK
fn make_contact(pk_byte: u8, name: &str) -> Contact {
    let mut pk = [0u8; 32];
    pk[0] = pk_byte;
    let card = ContactCard::new(name);
    Contact::from_exchange(
        pk,
        card,
        SymmetricKey::generate(),
        vauchi_core::clock::SystemClock::shared().unix_seconds(),
    )
}

// Duress PIN: Setup and Auth Mode Switching
// Feature: resistance_features.feature @duress

/// @scenario: Setup duress PIN with app password
#[test]
fn test_duress_pin_requires_app_password_first() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    // Setup duress without app password should fail
    let result = vauchi.setup_duress_password("6789");
    assert!(
        result.is_err(),
        "Setting up duress PIN without app password must fail"
    );
}

/// @scenario: Setup duress PIN after app password
#[test]
fn test_duress_pin_setup_after_app_password() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    // Setup app password first
    vauchi.setup_app_password("1234").unwrap();
    assert!(vauchi.is_password_enabled().unwrap());

    // Setup duress PIN
    vauchi.setup_duress_password("6789").unwrap();
    assert!(vauchi.is_duress_enabled().unwrap());
}

/// @scenario: Authenticate with normal password switches to Normal mode
#[test]
fn test_authenticate_with_normal_password_enters_normal_mode() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();
    vauchi.setup_app_password("1234").unwrap();
    vauchi.setup_duress_password("6789").unwrap();

    let mode = vauchi.authenticate("1234").unwrap();
    assert_eq!(mode, AuthMode::Normal);
}

/// @scenario: Authenticate with duress PIN switches to Duress mode
#[test]
fn test_authenticate_with_duress_pin_enters_duress_mode() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();
    vauchi.setup_app_password("1234").unwrap();
    vauchi.setup_duress_password("6789").unwrap();

    let mode = vauchi.authenticate("6789").unwrap();
    assert_eq!(mode, AuthMode::Duress);
}

/// @scenario: Invalid password fails authentication
#[test]
fn test_authenticate_with_invalid_password_fails() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();
    vauchi.setup_app_password("1234").unwrap();

    let result = vauchi.authenticate("9999");
    assert!(result.is_err(), "Invalid password must fail authentication");
}

// Duress Alerts: Queue and Configuration
// Feature: resistance_features.feature @duress

/// @scenario: Duress alert is queued when duress PIN is entered
#[test]
fn test_duress_alert_queued_on_duress_auth() {
    let mut alice = Vauchi::in_memory().unwrap();
    alice.create_identity("Alice").unwrap();
    alice.setup_app_password("1234").unwrap();
    alice.setup_duress_password("6789").unwrap();

    // Configure duress alert settings (no recipients yet)
    let settings = DuressSettings {
        alert_contact_ids: vec![],
        alert_message: "Emergency".to_string(),
        include_location: false,
    };
    alice.save_duress_settings(&settings).unwrap();

    // Authenticate with duress PIN
    let mode = alice.authenticate("6789").unwrap();
    assert_eq!(mode, AuthMode::Duress);

    // ADR-032: duress alerts are covert card-update sends to configured
    // trusted contacts — there is no detectable local "duress event". With
    // no recipients configured, there is nothing to disguise-send, so no
    // covert traffic is queued. (The positive path — an alert queued to a
    // ratcheted trusted contact — is owned by core's safety-alert and
    // duress-unlock wiring tests.)
    assert_eq!(
        alice.pending_update_count().unwrap(),
        0,
        "no recipients configured -> no covert duress traffic queued"
    );
}

/// @scenario: Duress settings can be configured and retrieved
#[test]
fn test_duress_settings_save_and_load() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    let settings = DuressSettings {
        alert_contact_ids: vec!["contact1".to_string(), "contact2".to_string()],
        alert_message: "Emergency situation".to_string(),
        include_location: true,
    };

    vauchi.save_duress_settings(&settings).unwrap();

    let loaded = vauchi.load_duress_settings().unwrap();
    assert!(loaded.is_some(), "Settings must be retrievable after save");

    let loaded_settings = loaded.unwrap();
    assert_eq!(loaded_settings.alert_contact_ids.len(), 2);
    assert_eq!(loaded_settings.alert_message, "Emergency situation");
    assert!(loaded_settings.include_location);
}

/// @scenario: Duress mode can be disabled
#[test]
fn test_disable_duress() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();
    vauchi.setup_app_password("1234").unwrap();
    vauchi.setup_duress_password("6789").unwrap();

    assert!(vauchi.is_duress_enabled().unwrap());

    vauchi.disable_duress().unwrap();
    assert!(!vauchi.is_duress_enabled().unwrap());
}

// Hidden Contacts: Basic Operations
// Feature: resistance_features.feature @hidden

/// @scenario: Contacts can be hidden
#[test]
fn test_hide_contact() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    // Hide the contact
    vauchi.hide_contact(&bob_id).unwrap();

    // Verify contact is hidden
    let loaded = vauchi
        .storage()
        .contacts()
        .load_contact(&bob_id)
        .unwrap()
        .unwrap();
    assert!(
        loaded.is_hidden(),
        "Contact must be hidden after hide_contact"
    );
}

/// @scenario: Hidden contacts do not appear in list_contacts
#[test]
fn test_hidden_contacts_excluded_from_list() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    let charlie = make_contact(2, "Charlie");
    let charlie_id = charlie.id().to_string();
    vauchi.add_contact(charlie).unwrap();

    // Hide Bob
    vauchi.hide_contact(&bob_id).unwrap();

    // List contacts should only show Charlie
    let visible = vauchi.list_contacts().unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id(), &charlie_id);
}

/// @scenario: Hidden contacts can be retrieved via list_hidden_contacts
#[test]
fn test_list_hidden_contacts() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    let charlie = make_contact(2, "Charlie");
    let charlie_id = charlie.id().to_string();
    vauchi.add_contact(charlie).unwrap();

    let diana = make_contact(3, "Diana");
    let _diana_id = diana.id().to_string();
    vauchi.add_contact(diana).unwrap();

    // Hide Bob and Charlie
    vauchi.hide_contact(&bob_id).unwrap();
    vauchi.hide_contact(&charlie_id).unwrap();

    // List hidden contacts
    let hidden = vauchi.list_hidden_contacts().unwrap();
    assert_eq!(hidden.len(), 2);

    let hidden_ids: Vec<&str> = hidden.iter().map(|c| c.id()).collect();
    assert!(hidden_ids.contains(&bob_id.as_str()));
    assert!(hidden_ids.contains(&charlie_id.as_str()));
}

/// @scenario: Hidden contacts can be unhidden
#[test]
fn test_unhide_contact() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    // Hide then unhide
    vauchi.hide_contact(&bob_id).unwrap();
    vauchi.unhide_contact(&bob_id).unwrap();

    // Verify not hidden
    let loaded = vauchi
        .storage()
        .contacts()
        .load_contact(&bob_id)
        .unwrap()
        .unwrap();
    assert!(!loaded.is_hidden());

    // Verify appears in list_contacts
    let visible = vauchi.list_contacts().unwrap();
    assert_eq!(visible.len(), 1);
}

// Hidden Contacts: Duress Integration
// Feature: resistance_features.feature @duress @hidden

/// @scenario: Duress mode shows only decoy contacts
#[test]
fn test_duress_mode_shows_decoy_contacts_only() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    // Create a real contact
    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    // Add a decoy contact
    let decoy_card = ContactCard::new("Decoy Alice");
    vauchi
        .add_decoy_contact("decoy1", "Decoy Alice", &decoy_card)
        .unwrap();

    // List in Normal mode
    let normal_contacts = vauchi.list_contacts().unwrap();
    assert_eq!(normal_contacts.len(), 1);
    assert_eq!(normal_contacts[0].id(), &bob_id);

    // Setup password and duress
    vauchi.setup_app_password("1234").unwrap();
    vauchi.setup_duress_password("6789").unwrap();

    // Authenticate with duress PIN
    vauchi.authenticate("6789").unwrap();

    // List in Duress mode
    let duress_contacts = vauchi.list_contacts().unwrap();
    // Duress mode returns decoys as real contacts
    assert!(!duress_contacts.is_empty());
    // Verify real contact is NOT in the list (only decoys)
    for contact in &duress_contacts {
        assert_ne!(contact.id(), &bob_id);
    }
}

// Emergency Broadcast: Setup and Configuration
// Feature: resistance_features.feature @emergency

/// @scenario: Configure emergency broadcast with trusted contacts
#[test]
fn test_configure_emergency_broadcast() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    // Create trusted contacts
    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    let charlie = make_contact(2, "Charlie");
    let charlie_id = charlie.id().to_string();
    vauchi.add_contact(charlie).unwrap();

    // Configure emergency broadcast
    vauchi
        .configure_emergency_broadcast(
            vec![bob_id.clone(), charlie_id.clone()],
            "Emergency - help needed".to_string(),
            false,
        )
        .unwrap();

    // Verify configuration
    let config = vauchi.load_emergency_config().unwrap();
    assert!(config.is_some());

    let config = config.unwrap();
    assert_eq!(config.trusted_contact_ids.len(), 2);
    assert_eq!(config.message, "Emergency - help needed");
    assert!(!config.include_location);
}

/// @scenario: Emergency broadcast respects max 10 trusted contacts
#[test]
fn test_emergency_broadcast_max_contacts() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    // Create 11 contacts (exceeds max of 10)
    let mut contact_ids = vec![];
    for i in 0..11 {
        let contact = make_contact(i as u8, &format!("Contact{}", i));
        contact_ids.push(contact.id().to_string());
        vauchi.add_contact(contact).unwrap();
    }

    // Try to configure with 11 trusted contacts — should fail
    let result = vauchi.configure_emergency_broadcast(contact_ids, "Emergency".to_string(), false);
    assert!(
        result.is_err(),
        "Must not allow more than 10 trusted contacts"
    );
}

/// @scenario: Send emergency broadcast queues for delivery
#[test]
fn test_send_emergency_broadcast_queues_for_delivery() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    // Trusted contact without an established ratchet — the genesis path's
    // target case (ADR-068, core!1432).
    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    // Configure emergency broadcast
    vauchi
        .configure_emergency_broadcast(
            vec![bob_id.clone()],
            "Emergency - help needed".to_string(),
            false,
        )
        .unwrap();

    let result = vauchi.send_emergency_broadcast().unwrap();

    // Session-less is no longer a skip: the alert rides a genesis envelope
    // rooted in the exchanged shared_key.
    assert_eq!(result.total, 1);
    assert_eq!(
        result.sent, 1,
        "a session-less trusted contact must receive a genesis-sealed alert"
    );
}

/// @scenario: Emergency broadcast configuration respects blocked contacts
#[test]
fn test_emergency_broadcast_honors_blocked_status() {
    let mut vauchi = Vauchi::in_memory().unwrap();
    vauchi.create_identity("Alice").unwrap();

    // Create two trusted contacts
    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    vauchi.add_contact(bob).unwrap();

    let charlie = make_contact(2, "Charlie");
    let charlie_id = charlie.id().to_string();
    vauchi.add_contact(charlie).unwrap();

    // Can configure emergency broadcast with both first
    vauchi
        .configure_emergency_broadcast(
            vec![bob_id.clone(), charlie_id.clone()],
            "Emergency".to_string(),
            false,
        )
        .unwrap();

    // Block Charlie after configuration
    vauchi.block_contact(&charlie_id).unwrap();

    // Send emergency broadcast — should skip Charlie (blocked)
    let result = vauchi.send_emergency_broadcast().unwrap();

    // Total = 2 contacts in config, sent <= 1 (Charlie skipped due to block)
    assert_eq!(result.total, 2);
    assert!(result.sent <= 1, "Blocked contacts should be skipped");
}

// Comprehensive: Duress with Hidden and Decoy Contacts
// Feature: resistance_features.feature @duress @hidden @comprehensive

/// @scenario: Full duress flow: real contacts hidden, decoys shown, alert queued
#[test]
fn test_duress_full_workflow() {
    let mut alice = Vauchi::in_memory().unwrap();
    alice.create_identity("Alice").unwrap();

    // Create real trusted contacts (hidden for duress)
    let bob = make_contact(1, "Bob");
    let bob_id = bob.id().to_string();
    alice.add_contact(bob).unwrap();

    let charlie = make_contact(2, "Charlie");
    let charlie_id = charlie.id().to_string();
    alice.add_contact(charlie).unwrap();

    // Hide real contacts
    alice.hide_contact(&bob_id).unwrap();
    alice.hide_contact(&charlie_id).unwrap();

    // Add decoy contacts
    let decoy_card = ContactCard::new("Decoy Alice Friend");
    alice
        .add_decoy_contact("decoy1", "Decoy Alice Friend", &decoy_card)
        .unwrap();
    alice
        .add_decoy_contact("decoy2", "Decoy Work Contact", &decoy_card)
        .unwrap();

    // Configure duress with alert to real hidden contacts
    let settings = DuressSettings {
        alert_contact_ids: vec![bob_id.clone(), charlie_id.clone()],
        alert_message: "Duress alarm".to_string(),
        include_location: true,
    };
    alice.save_duress_settings(&settings).unwrap();

    // Setup passwords
    alice.setup_app_password("1234").unwrap();
    alice.setup_duress_password("6789").unwrap();

    // Enter duress mode
    let mode = alice.authenticate("6789").unwrap();
    assert_eq!(mode, AuthMode::Duress);

    // Verify: decoys shown, real contacts hidden
    let visible = alice.list_contacts().unwrap();
    for contact in &visible {
        assert_ne!(contact.id(), &bob_id);
        assert_ne!(contact.id(), &charlie_id);
    }

    // Verify: hidden contacts not in list
    let hidden = alice.list_hidden_contacts().unwrap();
    let hidden_ids: Vec<&str> = hidden.iter().map(|c| c.id()).collect();
    assert!(hidden_ids.contains(&bob_id.as_str()));
    assert!(hidden_ids.contains(&charlie_id.as_str()));

    // ADR-032 + ADR-068: a trusted contact WITHOUT an established ratchet
    // still receives the covert alert — the send path seals a genesis
    // envelope rooted in the exchanged shared_key (pre-genesis this was
    // silently skipped, the life-safety gap closed by core!1432). Both
    // configured contacts must have a queued, wire-ordinary blob.
    assert_eq!(
        alice.pending_update_count().unwrap(),
        2,
        "a duress alert must queue a genesis-sealed blob per session-less trusted contact"
    );
}
