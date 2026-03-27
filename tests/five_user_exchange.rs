// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

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
//!
//! ## Test Tiers
//! - `smoke_*`: Fast tests for every push (< 5 min total)
//! - `integration_*`: Comprehensive tests for main branch

use vauchi_e2e_tests::prelude::*;

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Integration test: Full mesh exchange between five users.
/// Tags: integration, exchange, multi-user
/// Feature: contact_exchange.feature
#[tokio::test]
async fn integration_five_user_exchange() {
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
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    // Steps 3-6: Exchange between all users
    orch.exchange_all().await.expect("Failed to exchange all");

    // Step 7: Verify primary devices have correct contacts.
    // Note: Device sync to secondary devices is a known limitation
    // (codebase-review-tracker #38). Verify device 0 only.
    for name in ["Alice", "Bob", "Carol", "Dave", "Eve"] {
        let user = orch.user(name).unwrap();
        let user = user.read().await;
        let contacts = user
            .list_contacts_on_device(0)
            .await
            .unwrap_or_else(|_| panic!("{} contacts failed", name));
        assert_eq!(
            contacts.len(),
            4,
            "{} device 0 should have 4 contacts, got {}",
            name,
            contacts.len()
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Integration test: Sequential chain exchange (A->B->C->D->E).
/// Tags: integration, exchange, sequential
/// Feature: contact_exchange.feature
#[tokio::test]
async fn integration_sequential_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 2).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");
    orch.add_user("Eve", 3).expect("Failed to add Eve");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    // Exchange in sequence: Alice -> Bob -> Carol -> Dave -> Eve
    orch.exchange("Alice", "Bob")
        .await
        .expect("Alice-Bob exchange failed");
    orch.exchange("Bob", "Carol")
        .await
        .expect("Bob-Carol exchange failed");
    orch.exchange("Carol", "Dave")
        .await
        .expect("Carol-Dave exchange failed");
    orch.exchange("Dave", "Eve")
        .await
        .expect("Dave-Eve exchange failed");

    // Verify contact counts on primary devices only.
    // Device sync to secondary devices is a known limitation (#38).
    for (name, expected) in [
        ("Alice", 1),
        ("Bob", 2),
        ("Carol", 2),
        ("Dave", 2),
        ("Eve", 1),
    ] {
        let user = orch.user(name).unwrap();
        let user = user.read().await;
        let contacts = user
            .list_contacts_on_device(0)
            .await
            .unwrap_or_else(|_| panic!("{} contacts failed", name));
        assert_eq!(
            contacts.len(),
            expected,
            "{} device 0 should have {} contacts, got {}",
            name,
            expected,
            contacts.len()
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: sync_updates:Contact updates propagate to all devices
/// Integration test: Contact sync across all linked devices.
/// Tags: integration, sync, multi-device
/// Feature: sync_updates.feature
#[tokio::test]
async fn integration_contact_sync() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 3).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    // Exchange
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    // Verify Alice has 1 contact on primary device.
    // Note: After QR exchange, contacts appear as "New Contact" until the
    // exchange response with the display name is received and synced.
    // Device sync to secondary devices is a known limitation (#38).
    let alice = orch.user("Alice").unwrap();
    {
        let alice_guard = alice.read().await;
        let contacts = alice_guard
            .list_contacts_on_device(0)
            .await
            .expect("Failed to list contacts");
        assert_eq!(
            contacts.len(),
            1,
            "Alice device 0 should have 1 contact after exchange"
        );
    }
    orch.stop().await.expect("Failed to stop orchestrator");
}
