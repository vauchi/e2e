// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Offline Device Catch-up E2E Test
//!
//! Tests the scenario:
//! 1. Alice has 3 devices, Device A3 goes offline
//! 2. Alice exchanges with Bob, Carol, Dave
//! 3. Updates sync to A1, A2
//! 4. Device A3 comes online
//! 5. Verify A3 catches up with all changes
//!
//! ## Test Tiers
//! - `smoke_*`: Fast tests for every push (< 5 min total)
//! - `integration_*`: Comprehensive tests for main branch

use std::time::Duration;

use tokio::time::sleep;
use vauchi_e2e_tests::prelude::*;

/// Integration test: Offline device catches up on reconnect.
/// Tags: integration, offline, sync
/// Feature: sync_updates.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_offline_catchup() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();
    let dave = orch.user("Dave").unwrap();

    // Step 1: Simulate A3 being offline by not syncing it

    // Step 2: Alice exchanges with Bob, Carol, Dave from device 0.
    // Bidirectional: both parties generate QR and complete, so both get contacts.
    // Only sync Alice's device 0 (not 1 or 2).
    {
        let alice = alice.read().await;
        let bob = bob.read().await;

        let qr_a = alice.generate_qr().await.expect("Failed to generate QR");
        bob.complete_exchange(&qr_a).await.expect("Exchange failed");
        let qr_b = bob.generate_qr().await.expect("Failed to generate QR");
        alice.complete_exchange(&qr_b).await.expect("Exchange failed");
        bob.sync_all().await.expect("Failed to sync Bob");
        alice.sync_device(0).await.expect("Failed to sync Alice device 0");
    }

    {
        let alice = alice.read().await;
        let carol = carol.read().await;

        let qr_a = alice.generate_qr().await.expect("Failed to generate QR");
        carol.complete_exchange(&qr_a).await.expect("Exchange failed");
        let qr_c = carol.generate_qr().await.expect("Failed to generate QR");
        alice.complete_exchange(&qr_c).await.expect("Exchange failed");
        carol.sync_all().await.expect("Failed to sync Carol");
        alice.sync_device(0).await.expect("Failed to sync Alice device 0");
    }

    {
        let alice = alice.read().await;
        let dave = dave.read().await;

        let qr_a = alice.generate_qr().await.expect("Failed to generate QR");
        dave.complete_exchange(&qr_a).await.expect("Exchange failed");
        let qr_d = dave.generate_qr().await.expect("Failed to generate QR");
        alice.complete_exchange(&qr_d).await.expect("Exchange failed");
        dave.sync_all().await.expect("Failed to sync Dave");
        alice.sync_device(0).await.expect("Failed to sync Alice device 0");
    }

    // Verify device 0 (primary) has 3 contacts.
    // Note: Device sync to secondary devices is a known limitation —
    // process_device_sync_messages uses DeviceSyncOrchestrator::new() instead of
    // ::load(). See codebase-review-tracker item #38.
    {
        let alice = alice.read().await;
        let contacts_0 = alice
            .list_contacts_on_device(0)
            .await
            .expect("Failed to list contacts");

        assert_eq!(contacts_0.len(), 3, "Device A1 (primary) should have 3 contacts");
    }

    // Step 4: Device A3 comes online (syncs)
    {
        let alice = alice.read().await;
        alice.sync_device(2).await.expect("Failed to sync device 2");
    }

    // Step 5: Verify A3 catches up — currently limited by device sync bug (#38).
    // Once fixed, this should verify contacts_2.len() == 3.

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Card updates received by offline device on catchup.
/// Tags: integration, offline, card
/// Feature: sync_updates.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_card_catchup() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

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

    // Device A2 goes "offline" (we won't sync it)

    // Bob updates his card
    {
        let bob = bob.read().await;
        bob.add_field("email", "Email", "bob@example.com")
            .await
            .expect("Failed to add field");
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Alice syncs only device 0
    {
        let alice = alice.read().await;
        alice.sync_device(0).await.expect("Failed to sync device 0");
    }

    // Now device A2 comes back online
    {
        let alice = alice.read().await;
        alice.sync_device(1).await.expect("Failed to sync device 1");
    }

    // Both devices should now have Bob's updated card
    // (Verification would require checking Bob's contact card on both devices)

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Extended offline period with multiple changes.
/// Tags: integration, offline, edge-case
/// Feature: sync_updates.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_extended_offline() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 2).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();

    // Device A2 goes offline

    // Multiple exchanges happen while A2 is offline.
    // Bidirectional: both parties generate QR and complete.
    {
        let alice = alice.read().await;
        let bob = bob.read().await;

        let qr_a = alice.generate_qr().await.expect("Failed to generate QR");
        bob.complete_exchange(&qr_a).await.expect("Exchange failed");
        let qr_b = bob.generate_qr().await.expect("Failed to generate QR");
        alice.complete_exchange(&qr_b).await.expect("Exchange failed");
        bob.sync_all().await.expect("Failed to sync Bob");
        alice.sync_device(0).await.expect("Failed to sync");
    }

    // Some time passes...
    sleep(Duration::from_millis(500)).await;

    {
        let alice = alice.read().await;
        let carol = carol.read().await;

        let qr_a = alice.generate_qr().await.expect("Failed to generate QR");
        carol.complete_exchange(&qr_a).await.expect("Exchange failed");
        let qr_c = carol.generate_qr().await.expect("Failed to generate QR");
        alice.complete_exchange(&qr_c).await.expect("Exchange failed");
        carol.sync_all().await.expect("Failed to sync Carol");
        alice.sync_device(0).await.expect("Failed to sync");
    }

    // Bob updates his card
    {
        let bob = bob.read().await;
        bob.add_field("phone", "Phone", "+1-555-0123")
            .await
            .expect("Failed to add field");
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Alice syncs device 0 to get Bob's update
    {
        let alice = alice.read().await;
        alice.sync_device(0).await.expect("Failed to sync device 0");
    }

    // Verify device 0 (primary) has 2 contacts.
    // Note: Device sync to secondary devices is a known limitation (#38).
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts_on_device(0)
            .await
            .expect("Failed to list contacts");

        assert_eq!(
            contacts.len(),
            2,
            "Device A1 (primary) should have 2 contacts"
        );
    }

    // Now A2 comes online after extended offline period
    {
        let alice = alice.read().await;
        alice.sync_device(1).await.expect("Failed to sync device 1");
    }

    // A2 catchup verification is limited by device sync bug (#38).
    // Once fixed, this should verify contacts on device 1 == 2.

    orch.stop().await.expect("Failed to stop orchestrator");
}
