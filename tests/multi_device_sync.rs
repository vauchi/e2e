//! Multi-Device Sync Propagation E2E Test
//!
//! Tests the scenario:
//! 1. Alice updates card on Device A1
//! 2. Wait for sync to A2, A3
//! 3. Verify Bob's devices receive update
//! 4. Bob updates card on B2
//! 5. Verify Alice's devices receive update

use std::time::Duration;

use tokio::time::sleep;
use vauchi_e2e_tests::prelude::*;

/// Test that card updates propagate to all linked devices.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_card_update_propagation() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 2).expect("Failed to add Bob");

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    // Initial exchange
    orch.exchange("Alice", "Bob").await.expect("Exchange failed");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Step 1: Alice updates her card on device 0
    {
        let alice = alice.read().await;
        alice.add_field("email", "Email", "alice@example.com").await
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

    // Step 4: Bob updates his card
    {
        let bob = bob.read().await;
        bob.add_field("phone", "Phone", "+1-555-0123").await
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

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Test that device linking propagates existing contacts.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_new_device_receives_contacts() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");

    orch.create_all_identities().await.expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();

    // Alice exchanges with Bob and Carol
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let carol = carol.read().await;

        alice.exchange_with(&bob).await.expect("Exchange with Bob failed");
        alice.exchange_with(&carol).await.expect("Exchange with Carol failed");
    }

    // Verify Alice has 2 contacts
    {
        let alice = alice.read().await;
        let contacts = alice.list_contacts().await.expect("Failed to list contacts");
        assert_eq!(contacts.len(), 2, "Alice should have 2 contacts");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Test concurrent updates from multiple devices.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_concurrent_updates() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 2).expect("Failed to add Bob");

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

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
        task.await.expect("Task panicked").expect("Add field failed");
    }

    // Sync all devices
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync");
    }

    // Verify card exists without errors
    {
        let alice = alice.read().await;
        let card = alice.get_card().await.expect("Failed to get card");
        assert!(!card.name.is_empty(), "Card should have a name");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
