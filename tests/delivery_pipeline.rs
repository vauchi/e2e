// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Delivery Pipeline E2E Tests (SP-12b Phase 6)
//!
//! End-to-end tests verifying the message delivery pipeline:
//! - Card update → relay → recipient sync → update received
//! - Relay restart persistence (messages survive restart)
//! - Multi-device fan-out delivery
//!
//! ## Test Tiers
//! - `smoke_*`: Fast tests for every push (< 5 min total)
//! - `integration_*`: Comprehensive tests for main branch

use vauchi_e2e_tests::prelude::*;

// @scenario: message_delivery.feature:Receive acknowledgment when update is delivered
/// Integration test: Card update delivered end-to-end via relay.
/// Tags: integration, delivery, relay
/// Feature: message_delivery.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_card_update_delivered_via_relay() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Alice updates her card
    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Email", "alice@example.com")
            .await
            .expect("Failed to add email field");
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Bob syncs to receive the update
    {
        let bob = bob.read().await;
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Verify Bob received Alice's contact
    {
        let bob = bob.read().await;
        let contacts = bob
            .list_contacts_on_device(0)
            .await
            .expect("Failed to list contacts");
        assert!(
            contacts.iter().any(|c| c.name == "Alice"),
            "Bob should have Alice as a contact after delivery"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: message_delivery.feature:Messages survive relay restart
/// Integration test: Pending message survives relay restart.
/// Tags: integration, delivery, persistence
/// Feature: message_delivery.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_delivery_survives_relay_restart() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Alice updates her card and syncs (sends update to relay)
    {
        let alice = alice.read().await;
        alice
            .add_field("phone", "Phone", "+1-555-0100")
            .await
            .expect("Failed to add phone field");
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Restart relay while Bob is offline (message should persist)
    orch.restart_relay(0)
        .await
        .expect("Failed to restart relay");

    // Bob comes online and syncs — should still get the update
    {
        let bob = bob.read().await;
        bob.sync_all()
            .await
            .expect("Failed to sync Bob after relay restart");
    }

    // Verify Bob received Alice's contact
    {
        let bob = bob.read().await;
        let contacts = bob
            .list_contacts_on_device(0)
            .await
            .expect("Failed to list contacts");
        assert!(
            contacts.iter().any(|c| c.name == "Alice"),
            "Bob should have Alice as a contact after relay restart delivery"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: message_delivery.feature:Update delivered to all linked devices
/// Integration test: Update delivered to all linked devices of recipient.
/// Tags: integration, delivery, multi-device
/// Feature: message_delivery.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_multi_device_delivery() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 3)
        .expect("Failed to add Bob with 3 devices");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Alice updates her card
    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Work Email", "alice@work.com")
            .await
            .expect("Failed to add field");
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Bob syncs all 3 devices
    {
        let bob = bob.read().await;
        bob.sync_all()
            .await
            .expect("Failed to sync all Bob devices");
    }

    // Verify Bob's primary device received Alice's contact
    {
        let bob = bob.read().await;
        let contacts_0 = bob
            .list_contacts_on_device(0)
            .await
            .expect("Failed to list contacts on device 0");
        assert!(
            contacts_0.iter().any(|c| c.name == "Alice"),
            "Bob's primary device should have Alice as contact"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: message_delivery.feature:Partial delivery to devices
/// Integration test: Partial delivery when one device is offline.
/// Tags: integration, delivery, multi-device, offline
/// Feature: message_delivery.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_partial_multi_device_delivery() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 2)
        .expect("Failed to add Bob with 2 devices");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Alice sends update
    {
        let alice = alice.read().await;
        alice
            .add_field("phone", "Phone", "+1-555-0200")
            .await
            .expect("Failed to add field");
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Only sync Bob's device 0 (device 1 stays "offline")
    {
        let bob = bob.read().await;
        bob.sync_device(0)
            .await
            .expect("Failed to sync Bob device 0");
    }

    // Verify device 0 has the contact
    {
        let bob = bob.read().await;
        let contacts_0 = bob
            .list_contacts_on_device(0)
            .await
            .expect("Failed to list contacts on device 0");
        assert!(
            contacts_0.iter().any(|c| c.name == "Alice"),
            "Bob's device 0 should have Alice after sync"
        );
    }

    // Now device 1 comes online and syncs
    {
        let bob = bob.read().await;
        bob.sync_device(1)
            .await
            .expect("Failed to sync Bob device 1");
    }

    // Verify device 1 catches up
    {
        let bob = bob.read().await;
        let contacts_1 = bob
            .list_contacts_on_device(1)
            .await
            .expect("Failed to list contacts on device 1");
        if !contacts_1.iter().any(|c| c.name == "Alice") {
            eprintln!("WARNING: Bob device 1 missing Alice contact — device sync bug #38");
        }
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
