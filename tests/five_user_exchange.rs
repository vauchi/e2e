//! Five-User Full Exchange E2E Test
//!
//! Tests the scenario:
//! 1. Create identities for Alice(3), Bob(2), Carol(1), Dave(1), Eve(3)
//! 2. Link all secondary devices
//! 3. Alice exchanges with Bob, Carol, Dave, Eve
//! 4. Bob exchanges with Carol, Dave, Eve
//! 5. Carol exchanges with Dave, Eve
//! 6. Dave exchanges with Eve
//! 7. Verify all devices have correct contacts after sync

use vauchi_e2e_tests::prelude::*;

/// Test that all five users can exchange contacts and sync across all their devices.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_five_user_full_exchange() {
    // Setup orchestrator
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Create users with devices
    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 2).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");
    orch.add_user("Eve", 3).expect("Failed to add Eve");

    // Step 1 & 2: Create identities and link devices
    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    // Steps 3-6: Exchange between all users
    orch.exchange_all().await.expect("Failed to exchange all");

    // Step 7: Verify all devices have correct contacts
    // Each user should have 4 contacts (everyone else)
    for name in ["Alice", "Bob", "Carol", "Dave", "Eve"] {
        orch.verify_contact_count(name, 4).await
            .expect(&format!("{} should have 4 contacts", name));
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Test exchange in a specific order (sequential pairing).
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_sequential_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 2).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");
    orch.add_user("Eve", 3).expect("Failed to add Eve");

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    // Exchange in sequence: Alice -> Bob -> Carol -> Dave -> Eve
    orch.exchange("Alice", "Bob").await.expect("Alice-Bob exchange failed");
    orch.exchange("Bob", "Carol").await.expect("Bob-Carol exchange failed");
    orch.exchange("Carol", "Dave").await.expect("Carol-Dave exchange failed");
    orch.exchange("Dave", "Eve").await.expect("Dave-Eve exchange failed");

    // Verify contact counts
    orch.verify_contact_count("Alice", 1).await.expect("Alice should have 1 contact");
    orch.verify_contact_count("Bob", 2).await.expect("Bob should have 2 contacts");
    orch.verify_contact_count("Carol", 2).await.expect("Carol should have 2 contacts");
    orch.verify_contact_count("Dave", 2).await.expect("Dave should have 2 contacts");
    orch.verify_contact_count("Eve", 1).await.expect("Eve should have 1 contact");

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Test that contacts sync correctly across all devices.
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn test_contact_sync_across_devices() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    // Exchange
    orch.exchange("Alice", "Bob").await.expect("Exchange failed");

    // Verify Alice has Bob as contact on all devices
    let alice = orch.user("Alice").unwrap();
    let alice_guard = alice.read().await;

    for i in 0..alice_guard.device_count() {
        let contacts = alice_guard.list_contacts_on_device(i).await
            .expect("Failed to list contacts");
        assert!(
            contacts.iter().any(|c| c.name.contains("Bob")),
            "Alice's device {} should have Bob as contact",
            i
        );
    }

    drop(alice_guard);
    orch.stop().await.expect("Failed to stop orchestrator");
}
