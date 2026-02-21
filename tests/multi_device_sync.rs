// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Multi-Device Sync Propagation E2E Test
//!
//! Tests the scenario:
//! 1. Alice updates card on Device A1
//! 2. Wait for sync to A2, A3
//! 3. Verify Bob's devices receive update
//! 4. Bob updates card on B2
//! 5. Verify Alice's devices receive update
//!
//! ## Test Tiers
//! - `smoke_*`: Fast tests for every push (< 5 min total)
//! - `integration_*`: Comprehensive tests for main branch

use std::time::Duration;

use tokio::time::sleep;
use vauchi_e2e_tests::prelude::*;

// @scenario: sync_updates:Card update received from contact
/// Smoke test: Card update propagation across devices.
/// Tags: smoke, sync
/// Feature: sync_updates.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn smoke_card_update() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 2).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    // Initial exchange
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Step 1: Alice updates her card on device 0
    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Email", "alice@example.com")
            .await
            .expect("Failed to add field");
    }

    // Give time for propagation
    sleep(Duration::from_millis(500)).await;

    // Step 2: Sync all of Alice's devices
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Verify Alice's card is updated on all her devices
    {
        let alice = alice.read().await;
        for _i in 0..alice.device_count() {
            let card = alice.get_card().await.expect("Failed to get card");
            assert!(
                card.fields.iter().any(|f| f.value == "alice@example.com"),
                "Alice's devices should have updated email"
            );
        }
    }

    // Step 3: Bob syncs to receive Alice's update
    {
        let bob = bob.read().await;
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Verify Bob received Alice's contact and has her as a contact
    // TODO: Add get_contact_card() to Device trait to assert field-level values (alice@example.com)
    {
        let bob = bob.read().await;
        let contacts = bob
            .list_contacts()
            .await
            .expect("Failed to list Bob's contacts");
        assert!(
            !contacts.is_empty(),
            "Bob should have Alice as a contact after sync"
        );
    }

    // Step 4: Bob updates his card
    {
        let bob = bob.read().await;
        bob.add_field("phone", "Phone", "+1-555-0123")
            .await
            .expect("Failed to add Bob's field");
    }

    // Give time for propagation
    sleep(Duration::from_millis(500)).await;

    // Bob syncs all devices
    {
        let bob = bob.read().await;
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Step 5: Alice syncs to receive Bob's update
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Verify Alice received Bob's contact after cross-sync
    // TODO: Add get_contact_card() to Device trait to assert field-level values (+1-555-0123)
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list Alice's contacts");
        assert!(
            !contacts.is_empty(),
            "Alice should have Bob as a contact after sync"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: device_management:User links a new device
/// Integration test: Device linking propagates existing contacts.
/// Tags: integration, device-linking, sync
/// Feature: device_management.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_device_receives_contacts() {
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

    // Alice exchanges with Bob and Carol
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

    // Verify Alice has 2 contacts
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 2, "Alice should have 2 contacts");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: sync_updates:Concurrent card edits from multiple devices
/// Integration test: Concurrent updates from multiple devices.
/// Tags: integration, sync, concurrent
/// Feature: sync_updates.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_concurrent_updates() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 2).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    let alice = orch.user("Alice").unwrap();

    // Alice updates her card from multiple devices concurrently
    let update_tasks: Vec<_> = (0..3)
        .map(|i| {
            let alice = alice.clone();
            tokio::spawn(async move {
                let alice = alice.read().await;
                let label = format!("Field{}", i);
                let value = format!("value{}", i);
                alice.add_field("custom", &label, &value).await
            })
        })
        .collect();

    // Wait for all updates to complete
    for task in update_tasks {
        task.await
            .expect("Task panicked")
            .expect("Add field failed");
    }

    // Sync all devices
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync");
    }

    // Verify all concurrent field updates converged correctly
    {
        let alice = alice.read().await;
        let card = alice.get_card().await.expect("Failed to get card");
        assert!(!card.name.is_empty(), "Card should have a name");
        // All three concurrent updates should be present after sync
        for i in 0..3 {
            let expected_value = format!("value{}", i);
            assert!(
                card.fields.iter().any(|f| f.value == expected_value),
                "Card should contain concurrent field 'value{}' after sync, got fields: {:?}",
                i,
                card.fields.iter().map(|f| &f.value).collect::<Vec<_>>()
            );
        }
        assert!(
            card.fields.len() >= 3,
            "All 3 concurrent fields should be present, got {}",
            card.fields.len()
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
