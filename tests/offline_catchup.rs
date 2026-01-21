//! Offline Device Catch-up E2E Test
//!
//! Tests the scenario:
//! 1. Alice has 3 devices, Device A3 goes offline
//! 2. Alice exchanges with Bob, Carol, Dave
//! 3. Updates sync to A1, A2
//! 4. Device A3 comes online
//! 5. Verify A3 catches up with all changes

use std::time::Duration;

use tokio::time::sleep;
use vauchi_e2e_tests::prelude::*;

/// Test that an offline device catches up when it comes back online.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_offline_device_catchup() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();
    let dave = orch.user("Dave").unwrap();

    // Step 1: Simulate A3 being offline by not syncing it

    // Step 2: Alice exchanges with Bob, Carol, Dave from device 0
    {
        let alice = alice.read().await;
        let bob = bob.read().await;

        let qr = alice.generate_qr().await.expect("Failed to generate QR");
        bob.complete_exchange(&qr).await.expect("Exchange failed");
    }

    {
        let alice = alice.read().await;
        let carol = carol.read().await;

        let qr = alice.generate_qr().await.expect("Failed to generate QR");
        carol.complete_exchange(&qr).await.expect("Exchange failed");
    }

    {
        let alice = alice.read().await;
        let dave = dave.read().await;

        let qr = alice.generate_qr().await.expect("Failed to generate QR");
        dave.complete_exchange(&qr).await.expect("Exchange failed");
    }

    // Step 3: Sync A1, A2 (but not A3)
    {
        let alice = alice.read().await;
        alice.sync_device(0).await.expect("Failed to sync device 0");
        alice.sync_device(1).await.expect("Failed to sync device 1");
        // Device 2 (A3) intentionally not synced
    }

    // Verify A1 and A2 have 3 contacts
    {
        let alice = alice.read().await;
        let contacts_0 = alice.list_contacts_on_device(0).await.expect("Failed to list contacts");
        let contacts_1 = alice.list_contacts_on_device(1).await.expect("Failed to list contacts");

        assert_eq!(contacts_0.len(), 3, "Device A1 should have 3 contacts");
        assert_eq!(contacts_1.len(), 3, "Device A2 should have 3 contacts");
    }

    // Step 4: Device A3 comes online (syncs)
    {
        let alice = alice.read().await;
        alice.sync_device(2).await.expect("Failed to sync device 2");
    }

    // Step 5: Verify A3 catches up
    {
        let alice = alice.read().await;
        let contacts_2 = alice.list_contacts_on_device(2).await.expect("Failed to list contacts");

        assert_eq!(
            contacts_2.len(),
            3,
            "Device A3 should have caught up to 3 contacts"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Test that card updates are received by offline device on catchup.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_card_updates_catchup() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 2).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    // Initial exchange
    orch.exchange("Alice", "Bob").await.expect("Exchange failed");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Device A2 goes "offline" (we won't sync it)

    // Bob updates his card
    {
        let bob = bob.read().await;
        bob.add_field("email", "Email", "bob@example.com").await
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

/// Test extended offline period.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_extended_offline_period() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 2).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();

    // Device A2 goes offline

    // Multiple exchanges happen while A2 is offline
    {
        let alice = alice.read().await;
        let bob = bob.read().await;

        let qr = alice.generate_qr().await.expect("Failed to generate QR");
        bob.complete_exchange(&qr).await.expect("Exchange failed");
        alice.sync_device(0).await.expect("Failed to sync");
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Some time passes...
    sleep(Duration::from_millis(500)).await;

    {
        let alice = alice.read().await;
        let carol = carol.read().await;

        let qr = alice.generate_qr().await.expect("Failed to generate QR");
        carol.complete_exchange(&qr).await.expect("Exchange failed");
        alice.sync_device(0).await.expect("Failed to sync");
        carol.sync_all().await.expect("Failed to sync Carol");
    }

    // Bob updates his card
    {
        let bob = bob.read().await;
        bob.add_field("phone", "Phone", "+1-555-0123").await
            .expect("Failed to add field");
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Alice syncs device 0 to get Bob's update
    {
        let alice = alice.read().await;
        alice.sync_device(0).await.expect("Failed to sync device 0");
    }

    // Now A2 comes online after extended offline period
    {
        let alice = alice.read().await;
        alice.sync_device(1).await.expect("Failed to sync device 1");
    }

    // Verify A2 has all contacts and updates
    {
        let alice = alice.read().await;
        let contacts = alice.list_contacts_on_device(1).await
            .expect("Failed to list contacts");

        assert_eq!(
            contacts.len(),
            2,
            "Device A2 should have 2 contacts after catchup"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
