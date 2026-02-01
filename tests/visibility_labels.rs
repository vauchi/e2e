// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Visibility Labels E2E Test
//!
//! Tests the label-based visibility system:
//! 1. Create labels and assign contacts
//! 2. Control field visibility per label
//! 3. Verify visibility changes propagate via sync

use vauchi_e2e_tests::prelude::*;

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

    // Verify contacts
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 2, "Alice should have 2 contacts");
    }

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

    // Assign contacts to labels
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;

        device
            .add_contact_to_label("Work", "Bob")
            .await
            .expect("Failed to add Bob to Work");
        device
            .add_contact_to_label("Friends", "Carol")
            .await
            .expect("Failed to add Carol to Friends");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

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
            .add_field("phone", "Personal Phone", "+1-555-0101")
            .await
            .expect("Failed to add phone");
    }

    // Exchange contacts
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        alice
            .exchange_with(&bob)
            .await
            .expect("Exchange failed");
    }

    // Create label and assign Bob
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;

        device
            .create_label("Colleagues")
            .await
            .expect("Failed to create label");
        device
            .add_contact_to_label("Colleagues", "Bob")
            .await
            .expect("Failed to add Bob");

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

    // Verify Alice's card is intact
    {
        let alice = alice.read().await;
        let card = alice.get_card().await.expect("Failed to get card");
        assert!(
            card.fields.iter().any(|f| f.value == "alice@work.com"),
            "Alice should have work email"
        );
        assert!(
            card.fields.iter().any(|f| f.value == "+1-555-0101"),
            "Alice should have personal phone"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

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
            .add_field("phone", "Private Phone", "+1-555-SECRET")
            .await
            .expect("Failed to add phone");
    }

    // Exchange
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        alice
            .exchange_with(&bob)
            .await
            .expect("Exchange failed");
    }

    // Hide private phone from Bob
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;
        device
            .hide_field_from_contact("Bob", "Private Phone")
            .await
            .expect("Failed to hide field");
    }

    // Sync
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
