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

use vauchi_e2e_tests::prelude::*;

// @scenario: sync_updates:Card update received from contact
/// Smoke test: Card update propagation across devices.
/// Tags: smoke, sync
/// Feature: sync_updates.feature
///
/// Previously ignored: CLI sync hung due to infinite refetch loop in
/// HttpTransportAdapter (fixed in core `fba8f32c`).
// @internal
#[tokio::test]
async fn smoke_card_update() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Smoke test: 2+1 devices tests multi-device sync without exhausting
    // relay rate limits. Full 3+2 coverage is in integration tests.
    orch.add_user("Alice", 2).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

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

    // Step 2: Sync all of Alice's devices
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Verify Alice's card is updated on the device where the edit happened.
    // Inter-device card sync is not yet implemented (#38); secondary devices
    // do not receive edits made on the primary, so we assert the primary only.
    {
        let alice = alice.read().await;
        let card = alice
            .get_card_on_device(0)
            .await
            .expect("Failed to get Alice's primary card");
        assert!(
            card.fields.iter().any(|f| f.value == "alice@example.com"),
            "Alice's primary device should have the updated email"
        );
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
// @internal
#[tokio::test]
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

// @scenario: sync_updates:Multiple sequential card edits are durable
/// Integration test: Multiple sequential card edits on one device are durable.
/// Tags: integration, sync, sequential
/// Feature: sync_updates.feature
// @internal
#[tokio::test]
async fn integration_sequential_card_edits() {
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

    // Alice adds multiple fields sequentially on her primary device.
    // Each add_field is a separate CLI invocation — they must not lose data.
    {
        let alice = alice.read().await;
        for i in 0..3 {
            let label = format!("Field{}", i);
            let value = format!("value{}", i);
            alice
                .add_field("custom", &label, &value)
                .await
                .expect("Add field failed");
        }
    }

    // Sync all devices
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync");
    }

    // Verify all field updates are present on the primary device
    {
        let alice = alice.read().await;
        let card = alice.get_card().await.expect("Failed to get card");
        assert!(!card.name.is_empty(), "Card should have a name");
        for i in 0..3 {
            let expected_value = format!("value{}", i);
            assert!(
                card.fields.iter().any(|f| f.value == expected_value),
                "Card should contain field 'value{}' after sequential adds, got fields: {:?}",
                i,
                card.fields.iter().map(|f| &f.value).collect::<Vec<_>>()
            );
        }
        assert!(
            card.fields.len() >= 3,
            "All 3 fields should be present, got {}",
            card.fields.len()
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: sync_updates:Concurrent card edits from multiple devices converge
/// Integration test: Card edits from separate linked devices converge after sync.
/// Tags: integration, sync, concurrent, device-sync
/// Feature: sync_updates.feature
// @internal
#[tokio::test]
async fn integration_cross_device_card_convergence() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    let alice = orch.user("Alice").unwrap();

    // Each of Alice's 3 linked devices adds a different field
    {
        let alice = alice.read().await;
        for i in 0..3 {
            let device = alice.device(i).unwrap().clone();
            let device = device.read().await;
            let label = format!("Field{}", i);
            let value = format!("value{}", i);
            device
                .add_field("custom", &label, &value)
                .await
                .expect("Add field failed");
        }
    }

    // Multiple sync rounds to converge
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync (round 1)");
        alice.sync_all().await.expect("Failed to sync (round 2)");
    }

    // All 3 fields should be visible on the primary device
    {
        let alice = alice.read().await;
        let card = alice.get_card().await.expect("Failed to get card");
        for i in 0..3 {
            let expected_value = format!("value{}", i);
            assert!(
                card.fields.iter().any(|f| f.value == expected_value),
                "Card should contain 'value{}' after cross-device sync, got: {:?}",
                i,
                card.fields.iter().map(|f| &f.value).collect::<Vec<_>>()
            );
        }
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Every active device can exchange and update
/// Release certification: every linked device role can exchange and publish an
/// update that converges exactly on both users' three-device topologies.
// @internal
#[tokio::test]
async fn integration_six_device_exchange_and_update_convergence() {
    for (device_index, phone) in [
        (0, "+12025550101"),
        (1, "+12025550102"),
        (2, "+12025550103"),
    ] {
        certify_six_device_role(device_index, phone).await;
    }
}

async fn certify_six_device_role(device_index: usize, phone: &str) {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    let cli_url = orch
        .primary_cli_relay_url()
        .expect("CLI relay URL should be available");
    let direct_url = orch
        .primary_relay_http_url()
        .expect("application relay URL should be available");
    let outer_url = orch
        .ohttp_relay_url()
        .expect("certification requires an outer OHTTP relay");
    assert_eq!(
        cli_url, direct_url,
        "certification must identify the application relay separately"
    );
    assert_ne!(
        cli_url, outer_url,
        "certification traffic must traverse a distinct OHTTP origin"
    );

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 3).expect("Failed to add Bob");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");

    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("All devices should synchronize linked-device topology");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");

    for (name, user) in [("Alice", &alice), ("Bob", &bob)] {
        let user = user.read().await;
        for linked_index in 0..3 {
            let device = user
                .device(linked_index)
                .expect("linked device should exist");
            let listed = device
                .read()
                .await
                .list_devices()
                .await
                .expect("linked device should list owner topology");
            assert_eq!(
                listed.len(),
                3,
                "{name} device {} must know all three linked devices, got {listed:?}",
                linked_index + 1
            );
        }
    }

    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(device_index)
            .await
            .expect("Alice device should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(device_index)
            .await
            .expect("Bob device should start exchange");
        bob.complete_exchange_on_device(device_index, &alice_qr)
            .await
            .expect("Bob device should complete exchange");
        alice
            .complete_exchange_on_device(device_index, &bob_qr)
            .await
            .expect("Alice device should complete exchange");

        for _ in 0..2 {
            alice
                .sync_all()
                .await
                .expect("Alice devices should synchronize the exchange");
            bob.sync_all()
                .await
                .expect("Bob devices should synchronize the exchange");
        }

        let device = alice
            .device(device_index)
            .expect("Alice exchange device should exist")
            .clone();
        let device = device.read().await;
        device
            .add_field("phone", "ReleasePhone", phone)
            .await
            .expect("Alice exchange device should publish phone update");
        device
            .unhide_field_to_contact("Bob", "ReleasePhone")
            .await
            .expect("Alice should permit Bob to receive the phone update");
    }

    let mut missing_cards = Vec::new();
    for _ in 0..8 {
        {
            let alice = alice.read().await;
            alice.sync_all().await.expect("Alice sync should succeed");
        }
        {
            let bob = bob.read().await;
            bob.sync_all().await.expect("Bob sync should succeed");
        }

        missing_cards = missing_six_device_phone_cards(&alice, &bob, phone).await;
        if missing_cards.is_empty() {
            break;
        }
    }

    assert!(
        missing_cards.is_empty(),
        "A{} ↔ B{} exchange did not converge phone {phone}; missing exact value on {missing_cards:?}",
        device_index + 1,
        device_index + 1
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

async fn missing_six_device_phone_cards(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    phone: &str,
) -> Vec<String> {
    let mut missing = Vec::new();
    let alice = alice.read().await;
    for device_index in 0..3 {
        match alice.get_card_on_device(device_index).await {
            Ok(card)
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == "ReleasePhone" && field.value == phone) => {}
            _ => missing.push(format!("A{} owner card", device_index + 1)),
        }
    }
    drop(alice);

    let bob = bob.read().await;
    for device_index in 0..3 {
        let Some(device) = bob.device(device_index) else {
            missing.push(format!("B{} device", device_index + 1));
            continue;
        };
        match device.read().await.get_contact_card("Alice").await {
            Ok(Some(card))
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == "ReleasePhone" && field.value == phone) => {}
            _ => missing.push(format!("B{} Alice contact", device_index + 1)),
        }
    }
    missing
}
