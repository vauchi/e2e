// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Visibility Labels E2E Test
//!
//! Tests the label-based visibility system:
//! 1. Create labels and assign contacts
//! 2. Control field visibility per label
//! 3. Verify visibility changes propagate via sync
//!
//! Note: After mutual QR exchange, contacts appear as "New Contact"
//! until card updates are synced. Tests use contact ID prefixes
//! when name-based lookup is ambiguous.

use vauchi_e2e_tests::prelude::*;

// @scenario: visibility_labels:Create and assign visibility labels
/// Smoke test: Create labels and assign contacts.
/// Tags: smoke, visibility, labels
/// Feature: visibility_labels.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn smoke_create_labels_and_assign() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();

    // Exchange contacts
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let carol = carol.read().await;

        alice
            .exchange_with(&bob)
            .await
            .expect("Exchange with Bob failed");
        alice
            .exchange_with(&carol)
            .await
            .expect("Exchange with Carol failed");
    }

    // Verify contacts and collect ID prefixes for label assignment
    let (contact_id_1, contact_id_2) = {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 2, "Alice should have 2 contacts");

        // Use contact ID prefixes since both are named "New Contact"
        let id1 = contacts[0].id.clone().expect("Contact should have ID");
        let id2 = contacts[1].id.clone().expect("Contact should have ID");
        (id1, id2)
    };

    // Create labels on Alice's device
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;

        device
            .create_label("Work")
            .await
            .expect("Failed to create Work label");
        device
            .create_label("Friends")
            .await
            .expect("Failed to create Friends label");

        let labels = device.list_labels().await.expect("Failed to list labels");
        assert_eq!(labels.len(), 2, "Alice should have 2 labels");
    }

    // Assign contacts to labels using ID prefixes
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;

        device
            .add_contact_to_label("Work", &contact_id_1)
            .await
            .expect("Failed to add first contact to Work");
        device
            .add_contact_to_label("Friends", &contact_id_2)
            .await
            .expect("Failed to add second contact to Friends");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: visibility_labels:Label controls field visibility
/// Integration test: Field visibility controlled by labels.
/// Tags: integration, visibility, labels, sync
/// Feature: visibility_labels.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_label_visibility_sync() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Alice adds fields
    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Work Email", "alice@work.com")
            .await
            .expect("Failed to add work email");
        alice
            .add_field("phone", "Personal Phone", "+15550101")
            .await
            .expect("Failed to add phone");
    }

    // Exchange contacts
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        alice.exchange_with(&bob).await.expect("Exchange failed");
    }

    // Create label and assign contact (use "New Contact" — only one contact)
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;

        device
            .create_label("Colleagues")
            .await
            .expect("Failed to create label");
        device
            .add_contact_to_label("Colleagues", "New Contact")
            .await
            .expect("Failed to add contact to label");

        // Show work email, hide personal phone for Colleagues
        device
            .show_field_to_label("Colleagues", "Work Email")
            .await
            .expect("Failed to show field");
        device
            .hide_field_from_label("Colleagues", "Personal Phone")
            .await
            .expect("Failed to hide field");
    }

    // Sync
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    {
        let bob = bob.read().await;
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Verify Alice's card is intact (owner always sees all their own fields)
    {
        let alice = alice.read().await;
        let card = alice.get_card().await.expect("Failed to get card");
        assert!(
            card.fields.iter().any(|f| f.value == "alice@work.com"),
            "Alice should have work email"
        );
        assert!(
            card.fields.iter().any(|f| f.value == "+15550101"),
            "Alice should have personal phone"
        );
    }

    // Verify Bob has Alice as a contact after sync
    // TODO: Add get_contact_card() to Device trait to assert Bob sees work email
    //       but NOT personal phone (visibility filtering at field level)
    {
        let bob = bob.read().await;
        let contacts = bob
            .list_contacts()
            .await
            .expect("Failed to list Bob's contacts");
        assert!(
            !contacts.is_empty(),
            "Bob should have Alice as a contact after exchange and sync"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: visibility_control:Hide specific fields from individual contacts
/// Smoke test: Per-contact visibility hiding.
/// Tags: smoke, visibility, contacts
/// Feature: visibility_control.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn smoke_per_contact_visibility() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Alice adds fields
    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Public Email", "alice@public.com")
            .await
            .expect("Failed to add email");
        alice
            .add_field("phone", "Private Phone", "+15550199")
            .await
            .expect("Failed to add phone");
    }

    // Exchange
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        alice.exchange_with(&bob).await.expect("Exchange failed");
    }

    // Hide private phone from contact (use "New Contact" — only one contact)
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;
        device
            .hide_field_from_contact("New Contact", "Private Phone")
            .await
            .expect("Failed to hide field");
    }

    // Sync
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    {
        let bob = bob.read().await;
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Verify Alice's own card still has both fields (visibility is per-contact, not deletion)
    {
        let alice = alice.read().await;
        let card = alice.get_card().await.expect("Failed to get Alice's card");
        assert!(
            card.fields.iter().any(|f| f.value == "alice@public.com"),
            "Alice's own card should still contain Public Email"
        );
        assert!(
            card.fields.iter().any(|f| f.value == "+15550199"),
            "Alice's own card should still contain Private Phone (visibility is per-contact, not deletion)"
        );
    }

    // Verify Bob has Alice as a contact
    // TODO: Add get_contact_card() to Device trait to assert Bob sees Public Email
    //       but NOT Private Phone (per-contact visibility filtering)
    {
        let bob = bob.read().await;
        let contacts = bob
            .list_contacts()
            .await
            .expect("Failed to list Bob's contacts");
        assert!(
            !contacts.is_empty(),
            "Bob should have Alice as a contact after exchange and sync"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
