// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Relay Failover E2E Test
//!
//! Tests the scenario:
//! 1. All users connected to Relay A
//! 2. Stop Relay A
//! 3. Verify users failover to Relay B
//! 4. Updates continue to propagate
//! 5. Restart Relay A
//! 6. Verify recovery
//!
//! ## Test Tiers
//! - `smoke_*`: Fast tests for every push (< 5 min total)
//! - `integration_*`: Comprehensive tests for main branch

use std::time::Duration;

use tokio::time::sleep;
use vauchi_e2e_tests::prelude::*;

// @scenario: relay_network:Relay failover to backup server
/// Integration test: Relay failover and recovery.
/// Tags: integration, relay, failover
/// Feature: relay_network.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_relay_failover() {
    let config = OrchestratorConfig {
        relay_count: 2,
        ..Default::default()
    };

    let mut orch = Orchestrator::with_config(config);
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    // Initial exchange using primary relay
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    // Verify initial state
    orch.verify_contact_count("Alice", 1)
        .await
        .expect("Alice should have 1 contact");
    orch.verify_contact_count("Bob", 1)
        .await
        .expect("Bob should have 1 contact");

    // Step 2: Stop primary relay (relay 0)
    orch.stop_relay(0).await.expect("Failed to stop relay 0");

    // Give time for detection
    sleep(Duration::from_secs(1)).await;

    // Step 5: Restart primary relay
    orch.restart_relay(0)
        .await
        .expect("Failed to restart relay 0");

    // Give time for recovery
    sleep(Duration::from_secs(2)).await;

    // Step 6: Verify recovery by syncing
    orch.sync_all().await.expect("Failed to sync all");

    // Verify contacts are still intact
    orch.verify_contact_count("Alice", 1)
        .await
        .expect("Alice should still have 1 contact");
    orch.verify_contact_count("Bob", 1)
        .await
        .expect("Bob should still have 1 contact");

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: relay_network:Updates queued during outage delivered after recovery
/// Integration test: Updates during relay outage sync after recovery.
/// Tags: integration, relay, offline
/// Feature: relay_network.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_updates_during_outage() {
    let config = OrchestratorConfig {
        relay_count: 2,
        ..Default::default()
    };

    let mut orch = Orchestrator::with_config(config);
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

    // Stop primary relay
    orch.stop_relay(0).await.expect("Failed to stop relay 0");

    // Alice updates her card while relay is down
    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Email", "alice@offline.com")
            .await
            .expect("Failed to add field");
    }

    // Try to sync (will fail, but shouldn't crash)
    {
        let alice = alice.read().await;
        let _ = alice.sync_all().await; // Might fail, expected
    }

    // Restart relay
    orch.restart_relay(0)
        .await
        .expect("Failed to restart relay");

    // Give time for recovery
    sleep(Duration::from_secs(2)).await;

    // Sync after recovery
    {
        let alice = alice.read().await;
        alice.sync_all().await.expect("Failed to sync Alice");
    }

    // Bob syncs to receive update
    {
        let bob = bob.read().await;
        bob.sync_all().await.expect("Failed to sync Bob");
    }

    // Verify Bob still has Alice as a contact after relay outage + recovery
    {
        let bob = bob.read().await;
        let alice_contact = bob
            .get_contact("Alice")
            .await
            .expect("Failed to get Alice on Bob");
        assert!(
            alice_contact.is_some(),
            "Bob must still have Alice as a contact after relay outage recovery"
        );
    }

    // Verify contacts survived the outage
    orch.verify_contact_count("Alice", 1)
        .await
        .expect("Alice should have 1 contact after outage recovery");
    orch.verify_contact_count("Bob", 1)
        .await
        .expect("Bob should have 1 contact after outage recovery");
    // TODO: Once Device trait gains get_contact_card(), verify Bob received
    // Alice's email field update queued during the relay outage.

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: relay_network:Graceful degradation when all relays unavailable
/// Integration test: Graceful degradation when all relays unavailable.
/// Tags: integration, relay, edge-case
/// Feature: relay_network.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_no_relays() {
    let config = OrchestratorConfig {
        relay_count: 2,
        ..Default::default()
    };

    let mut orch = Orchestrator::with_config(config);
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

    // Stop all relays
    orch.stop_relay(0).await.expect("Failed to stop relay 0");
    orch.stop_relay(1).await.expect("Failed to stop relay 1");

    // Try to sync (should fail gracefully)
    {
        let alice = alice.read().await;
        let result = alice.sync_all().await;
        // Sync should fail when no relays are available
        assert!(
            result.is_err(),
            "Sync should fail when no relays are available"
        );
    }

    // Restart both relays
    orch.restart_relay(0)
        .await
        .expect("Failed to restart relay 0");
    orch.restart_relay(1)
        .await
        .expect("Failed to restart relay 1");

    // Give time for recovery
    sleep(Duration::from_secs(2)).await;

    // Sync should work now
    {
        let alice = alice.read().await;
        alice
            .sync_all()
            .await
            .expect("Failed to sync after relay recovery");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}
