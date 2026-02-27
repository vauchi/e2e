// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Contact Actions E2E Tests
//!
//! End-to-end validation of URI generation and security whitelist
//! across the full contact lifecycle: identity creation → card exchange →
//! field URI generation → security validation.
//!
//! These tests exercise the complete stack without needing relay or subprocess.
//!
//! Feature: contact_actions.feature

use vauchi_core::contact_card::ContactAction;
use vauchi_core::network::MockTransport;
use vauchi_core::{Contact, ContactField, FieldType, SymmetricKey, Vauchi};

/// Helper: create two users, exchange cards, return both instances.
fn exchange_with_fields(
    alice_fields: Vec<ContactField>,
    bob_fields: Vec<ContactField>,
) -> (Vauchi<MockTransport>, Vauchi<MockTransport>) {
    let mut alice = Vauchi::in_memory().unwrap();
    alice.create_identity("Alice").unwrap();
    for field in alice_fields {
        alice.add_own_field(field).unwrap();
    }

    let mut bob = Vauchi::in_memory().unwrap();
    bob.create_identity("Bob").unwrap();
    for field in bob_fields {
        bob.add_own_field(field).unwrap();
    }

    // Perform in-memory exchange
    let alice_card = alice.own_card().unwrap().unwrap();
    let bob_card = bob.own_card().unwrap().unwrap();

    let alice_pk = *alice.identity().unwrap().signing_public_key();
    let bob_pk = *bob.identity().unwrap().signing_public_key();

    // Simulate exchange: each side creates a Contact from the other's card
    let bob_contact = Contact::from_exchange(bob_pk, bob_card, SymmetricKey::generate());
    alice.add_contact(bob_contact).unwrap();

    let alice_contact = Contact::from_exchange(alice_pk, alice_card, SymmetricKey::generate());
    bob.add_contact(alice_contact).unwrap();

    (alice, bob)
}

// ============================================================
// E2E: Phone URI generation through exchange
// Feature: contact_actions.feature @phone @tel
// ============================================================

/// @scenario: contact_actions:Tap phone number opens dialer
#[test]
fn e2e_phone_field_generates_tel_uri_after_exchange() {
    let bob_fields = vec![ContactField::new(
        FieldType::Phone,
        "Mobile",
        "+41791234567",
    )];
    let (alice, _bob) = exchange_with_fields(vec![], bob_fields);

    let contacts = alice.list_contacts().unwrap();
    assert_eq!(contacts.len(), 1);

    let bob_contact = &contacts[0];
    let phone_field = &bob_contact.card().fields()[0];
    let uri = phone_field.to_uri();
    assert!(uri.is_some(), "phone field must generate a URI");
    assert!(
        uri.unwrap().starts_with("tel:"),
        "phone URI must use tel: scheme"
    );
}

/// @scenario: contact_actions:Long press phone number shows action menu
#[test]
fn e2e_phone_secondary_actions_include_call_sms_copy() {
    let bob_fields = vec![ContactField::new(
        FieldType::Phone,
        "Mobile",
        "+1-555-123-4567",
    )];
    let (alice, _bob) = exchange_with_fields(vec![], bob_fields);

    let contacts = alice.list_contacts().unwrap();
    let phone_field = &contacts[0].card().fields()[0];
    let actions = phone_field.to_secondary_actions();

    assert_eq!(actions.len(), 3, "phone should have Call, SendSms, Copy");
    assert!(matches!(&actions[0], ContactAction::Call(_)));
    assert!(matches!(&actions[1], ContactAction::SendSms(_)));
    assert!(matches!(&actions[2], ContactAction::CopyToClipboard));
}

// ============================================================
// E2E: Address directions URI
// Feature: contact_actions.feature @address @directions
// ============================================================

/// @scenario: contact_actions:Get directions to address
#[test]
fn e2e_address_field_generates_directions_uri() {
    let bob_fields = vec![ContactField::new(
        FieldType::Address,
        "Home",
        "Bahnhofstrasse 1, 8001 Zurich",
    )];
    let (alice, _bob) = exchange_with_fields(vec![], bob_fields);

    let contacts = alice.list_contacts().unwrap();
    let address_field = &contacts[0].card().fields()[0];

    let directions = address_field.to_directions_uri();
    assert!(directions.is_some(), "address must have directions URI");
    let uri = directions.unwrap();
    assert!(
        uri.contains("openstreetmap.org/directions"),
        "directions URI must use OpenStreetMap"
    );
    assert!(
        uri.contains("Bahnhofstrasse"),
        "directions URI must contain the address"
    );
}

// ============================================================
// E2E: Security — blocked URI schemes
// Feature: contact_actions.feature @security
// ============================================================

/// @scenario: contact_actions:URLs are validated before opening (#45)
#[test]
fn e2e_javascript_uri_blocked_after_exchange() {
    let bob_fields = vec![ContactField::new(
        FieldType::Website,
        "Malicious",
        "javascript:alert(1)",
    )];
    let (alice, _bob) = exchange_with_fields(vec![], bob_fields);

    let contacts = alice.list_contacts().unwrap();
    let field = &contacts[0].card().fields()[0];
    let uri = field.to_uri();

    assert!(
        uri.is_none(),
        "javascript: scheme must be blocked — got {:?}",
        uri
    );
}

/// @scenario: contact_actions:Only safe URI schemes are allowed (#46)
#[test]
fn e2e_file_uri_blocked_after_exchange() {
    let bob_fields = vec![ContactField::new(
        FieldType::Custom,
        "Exploit",
        "file:///etc/passwd",
    )];
    let (alice, _bob) = exchange_with_fields(vec![], bob_fields);

    let contacts = alice.list_contacts().unwrap();
    let field = &contacts[0].card().fields()[0];
    let uri = field.to_uri();

    assert!(
        uri.is_none(),
        "file: scheme must be blocked — got {:?}",
        uri
    );
}

/// @scenario: contact_actions:Allowed URI schemes whitelist (#47)
#[test]
fn e2e_allowed_schemes_work_after_exchange() {
    let bob_fields = vec![
        ContactField::new(FieldType::Phone, "Phone", "+41791234567"),
        ContactField::new(FieldType::Email, "Email", "bob@example.com"),
        ContactField::new(FieldType::Website, "Web", "https://example.com"),
        ContactField::new(FieldType::Address, "Home", "Zurich, Switzerland"),
    ];
    let (alice, _bob) = exchange_with_fields(vec![], bob_fields);

    let contacts = alice.list_contacts().unwrap();
    let fields = contacts[0].card().fields();

    // All fields must generate valid URIs
    for field in fields {
        let uri = field.to_uri();
        assert!(
            uri.is_some(),
            "field '{}' ({:?}) must generate a URI",
            field.label(),
            field.field_type()
        );

        let uri_str = uri.unwrap();
        let has_allowed_scheme = uri_str.starts_with("tel:")
            || uri_str.starts_with("mailto:")
            || uri_str.starts_with("https:")
            || uri_str.starts_with("http:")
            || uri_str.starts_with("geo:")
            || uri_str.starts_with("sms:");
        assert!(
            has_allowed_scheme,
            "field '{}' URI '{}' must use an allowed scheme",
            field.label(),
            uri_str
        );
    }
}

// ============================================================
// E2E: Social media Mastodon federation
// Feature: contact_actions.feature @social @profile-url
// ============================================================

/// @scenario: contact_actions:Tap social media opens profile (Mastodon federated)
#[test]
fn e2e_mastodon_federated_handle_generates_correct_url() {
    let bob_fields = vec![ContactField::new(
        FieldType::Social,
        "Mastodon",
        "@bob@fosstodon.org",
    )];
    let (alice, _bob) = exchange_with_fields(vec![], bob_fields);

    let contacts = alice.list_contacts().unwrap();
    let social_field = &contacts[0].card().fields()[0];
    let uri = social_field.to_uri();

    assert!(uri.is_some(), "Mastodon field must generate a URI");
    let uri_str = uri.unwrap();
    assert!(
        uri_str.contains("fosstodon.org"),
        "Mastodon federated handle must resolve to the user's instance, got: {}",
        uri_str
    );
    assert!(
        uri_str.contains("/@bob"),
        "Mastodon URI must contain the username path, got: {}",
        uri_str
    );
}
