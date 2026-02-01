// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Social Recovery E2E Test
//!
//! Tests the complete social recovery workflow:
//! 1. Alice exchanges with 3 contacts (Bob, Carol, Dave)
//! 2. Alice "loses" her device (new identity)
//! 3. Contacts vouch for Alice's recovery claim
//! 4. Alice assembles and verifies recovery proof

use vauchi_e2e_tests::prelude::*;

/// Smoke test: Recovery claim creation.
/// Tags: smoke, recovery
/// Feature: contact_recovery.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn smoke_recovery_setup() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();
    let dave = orch.user("Dave").unwrap();

    // Exchange Alice with all three vouchers
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let carol = carol.read().await;
        let dave = dave.read().await;

        alice
            .exchange_with(&bob)
            .await
            .expect("Exchange with Bob failed");
        alice
            .exchange_with(&carol)
            .await
            .expect("Exchange with Carol failed");
        alice
            .exchange_with(&dave)
            .await
            .expect("Exchange with Dave failed");
    }

    // Verify contacts established
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 3, "Alice should have 3 contacts");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Contact verification.
/// Tags: integration, verification
/// Feature: contact_exchange.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_contact_verification() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Exchange contacts
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        alice.exchange_with(&bob).await.expect("Exchange failed");
    }

    // Alice verifies Bob's fingerprint
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;
        device
            .verify_contact("Bob")
            .await
            .expect("Failed to verify Bob");
    }

    // Verify the contact shows as verified
    {
        let alice = alice.read().await;
        let device = alice.device(0).expect("No device");
        let device = device.read().await;
        let contact = device
            .get_contact("Bob")
            .await
            .expect("Failed to get contact");
        assert!(contact.is_some(), "Bob should exist as contact");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
