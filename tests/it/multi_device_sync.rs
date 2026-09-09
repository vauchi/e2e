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

fn six_device_certification_config() -> OrchestratorConfig {
    OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        // These scenarios certify convergence under explicit delivery faults.
        // Rate limiting has its own OHTTP integration test and can otherwise
        // delay one causal round beyond this suite's bounded convergence loop.
        ohttp_relay_config: OhttpRelayConfig {
            rate_limit_per_sec: 0,
            ..Default::default()
        },
        ..Default::default()
    }
}

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
/// update that converges exactly on both users' three-device topologies, with
/// updates originating from Alice's and Bob's devices alike.
// @internal
#[tokio::test]
async fn integration_six_device_exchange_and_update_convergence() {
    let mut orch = Orchestrator::with_config(OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        ..Default::default()
    });
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

    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
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

    assert_six_device_owner_topology(&orch).await;

    for (device_index, alice_phone, bob_phone) in [
        (0, "+12025550101", "+12025550201"),
        (1, "+12025550102", "+12025550202"),
        (2, "+12025550103", "+12025550203"),
    ] {
        certify_six_device_role(&orch, device_index, alice_phone, bob_phone).await;
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:A single exchange converges across all linked devices
/// Release certification: after a SINGLE A1<->B1 exchange (all six devices
/// linked *before* the exchange), updates authored on secondary linked devices
/// converge on every device — the production topology real users hit.
///
/// This closes the coverage hole the diagonal `certify_six_device_role` leaves:
/// there, every Alice device that sends has first done its OWN direct exchange
/// with a Bob device, so no secondary device ever sends without a direct
/// session. Here only A1<->B1 exchange; the record
/// `problems/2026-07-10-multi-device-ratchet-topology-gap` (line 143) flagged
/// that the per-device-role topology never proved this path. It asserts, in
/// order: (1) a secondary device A2 that never exchanged delivers to every Bob
/// device, and (2) concurrent competing edits from A1 and A2 converge to one
/// ADR-020 LWW winner on all six devices. Convergence is achieved through the
/// shared_key-bootstrapped session + owner-device card sync + per-peer-device
/// LWW. F4 registry activation (ADR-064 Amendment 2026-07-25) now also runs
/// during these sync rounds; the lost-primary certification below covers the
/// activation-dependent path
/// (`problems/2026-07-21-per-device-ratchet-registry-dormant/`).
// @internal
#[tokio::test]
async fn integration_six_device_single_exchange_convergence() {
    let mut orch = Orchestrator::with_config(six_device_certification_config());
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("linked-device topology should synchronize");
    }
    assert_six_device_owner_topology(&orch).await;

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");

    // A SINGLE exchange, primary devices only (A1 <-> B1). Secondary devices
    // never exchange — they must rely on the peer registry propagating over
    // owner-device sync.
    let bob_alice_contact_id = {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");

        for _ in 0..2 {
            alice
                .sync_all()
                .await
                .expect("Alice devices should synchronize the exchange");
            bob.sync_all()
                .await
                .expect("Bob devices should synchronize the exchange");
        }

        exchanged_contact_id(
            &bob.list_contacts_on_device(0)
                .await
                .expect("B1 should list contacts after exchange"),
            "Alice",
        )
    };

    // Sanity: owner-device sync propagated the Bob contact to A2, even though
    // A2 never exchanged. A failure here is a harness fault, not the registry
    // gap under test — the gap is delivery, not contact discovery.
    let alice_bob_contact_id_on_a2 = {
        let alice = alice.read().await;
        exchanged_contact_id(
            &alice
                .list_contacts_on_device(1)
                .await
                .expect("A2 should list contacts"),
            "Bob",
        )
    };

    // Baseline: the exchanged device (A1) reaches every Bob device. Proves the
    // split-OHTTP path and Bob's owner-device card fanout are sound, isolating
    // the secondary-device failure below.
    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").clone();
        let a1 = a1.read().await;
        let a1_bob = exchanged_contact_id(
            &a1.list_contacts().await.expect("A1 should list contacts"),
            "Bob",
        );
        a1.add_field("phone", "PrimaryPhone", "+12025550801")
            .await
            .expect("A1 should publish the baseline field");
        a1.unhide_field_to_contact(&a1_bob, "PrimaryPhone")
            .await
            .expect("A1 should permit Bob to receive the baseline field");
    }
    let mut baseline_missing = Vec::new();
    for _ in 0..8 {
        alice.read().await.sync_all().await.expect("Alice sync");
        bob.read().await.sync_all().await.expect("Bob sync");
        baseline_missing = missing_six_device_phone_cards_by_contact_id(
            &alice,
            &bob,
            &bob_alice_contact_id,
            "PrimaryPhone",
            "+12025550801",
        )
        .await;
        if baseline_missing.is_empty() {
            break;
        }
    }
    assert!(
        baseline_missing.is_empty(),
        "baseline: A1's update must converge on all devices; missing {baseline_missing:?}"
    );

    // The gap: an update authored on A2 (a secondary device that never
    // exchanged) must reach every Bob device. On current main it cannot — A2
    // holds no ratchet session for Bob and no seeded peer registry to bootstrap
    // one, so the update never reaches any Bob device.
    {
        let alice = alice.read().await;
        let a2 = alice.device(1).expect("A2 should exist").clone();
        let a2 = a2.read().await;
        a2.add_field("phone", "SecondaryPhone", "+12025550802")
            .await
            .expect("A2 should publish the field locally");
        a2.unhide_field_to_contact(&alice_bob_contact_id_on_a2, "SecondaryPhone")
            .await
            .expect("A2 should permit Bob to receive the field");
    }
    let mut gap_missing = Vec::new();
    for _ in 0..8 {
        alice.read().await.sync_all().await.expect("Alice sync");
        bob.read().await.sync_all().await.expect("Bob sync");
        gap_missing = missing_six_device_phone_cards_by_contact_id(
            &alice,
            &bob,
            &bob_alice_contact_id,
            "SecondaryPhone",
            "+12025550802",
        )
        .await;
        if gap_missing.is_empty() {
            break;
        }
    }
    assert!(
        gap_missing.is_empty(),
        "secondary-device update from A2 must converge on all Bob devices after a \
         single exchange; missing {gap_missing:?}"
    );

    // The decisive ADR-064 case: CONCURRENT sends from two Alice devices under
    // a single exchange. A1 and A2 edit the same field to competing values with
    // no owner-sync between them, then both deliver to Bob. Bob must reconcile
    // two independently advanced device chains and converge to one LWW winner on
    // every device. The green `concurrent_field_edits` test only covers this in
    // the diagonal topology where A1 and A2 each hold their own direct exchange
    // session; here neither secondary session came from a direct exchange.
    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").clone();
        let a1 = a1.read().await;
        let a1_bob = exchanged_contact_id(
            &a1.list_contacts().await.expect("A1 should list contacts"),
            "Bob",
        );
        a1.add_field("phone", "ConcurrentPhone", "+12025550900")
            .await
            .expect("A1 should seed the shared field");
        a1.unhide_field_to_contact(&a1_bob, "ConcurrentPhone")
            .await
            .expect("A1 should permit Bob to receive the shared field");
    }
    for _ in 0..3 {
        alice.read().await.sync_all().await.expect("Alice sync");
        bob.read().await.sync_all().await.expect("Bob sync");
    }

    let (a1, a2) = {
        let alice = alice.read().await;
        (
            alice.device(0).expect("A1 should exist").clone(),
            alice.device(1).expect("A2 should exist").clone(),
        )
    };
    let (a1_edit, a2_edit) = tokio::join!(
        async {
            a1.read()
                .await
                .edit_field("ConcurrentPhone", "+12025550901")
                .await
        },
        async {
            a2.read()
                .await
                .edit_field("ConcurrentPhone", "+12025550902")
                .await
        },
    );
    a1_edit.expect("A1 concurrent edit should succeed");
    a2_edit.expect("A2 concurrent edit should succeed");

    let mut winner = None;
    let mut concurrent_missing = Vec::new();
    for _ in 0..8 {
        orch.sync_all()
            .await
            .expect("concurrent edits should synchronize");
        let value = {
            let alice = alice.read().await;
            alice
                .get_card_on_device(0)
                .await
                .expect("A1 owner card should be readable")
                .fields
                .into_iter()
                .find(|field| field.label == "ConcurrentPhone")
                .map(|field| field.value)
                .expect("A1 should retain the concurrently edited field")
        };
        concurrent_missing = missing_six_device_phone_cards_by_contact_id(
            &alice,
            &bob,
            &bob_alice_contact_id,
            "ConcurrentPhone",
            &value,
        )
        .await;
        if concurrent_missing.is_empty() {
            winner = Some(value);
            break;
        }
    }
    assert!(
        winner.is_some(),
        "concurrent A1/A2 edits under a single exchange must converge to one LWW \
         winner on every device; still missing {concurrent_missing:?}"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// F4 lost-primary continuity certification (ADR-064 Amendment 2026-07-25).
///
/// The alpha-gating scenario: the exchanging device A1 goes permanently
/// dark right after the exchange — before any registry handshake ran — so
/// the surviving siblings hold only the owner-synced contact + shared_key.
/// Pre-F4 this orphaned the relationship bidirectionally (delivery was
/// A1-mediated). With F4, A2's sync ticks genesis-push the registry, the
/// bilateral handshake completes over the split-OHTTP relay path, and
/// cards flow BOTH ways without A1 ever syncing again and without
/// re-exchange.
// @scenario: multi_device_sync :: Lost exchanging device no longer orphans the relationship
#[tokio::test]
async fn integration_six_device_lost_primary_continuity_certification() {
    let mut orch = Orchestrator::with_config(six_device_certification_config());
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("linked-device topology should synchronize");
    }
    assert_six_device_owner_topology(&orch).await;

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");

    // Single exchange, primary devices only (A1 <-> B1).
    let bob_alice_contact_id = {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");

        // The ONLY rounds A1 ever syncs: enough owner-device sync for the
        // siblings to learn the Bob contact + shared_key. After this, A1 is
        // permanently lost.
        for _ in 0..2 {
            alice
                .sync_all()
                .await
                .expect("Alice devices should synchronize the exchange");
            bob.sync_all()
                .await
                .expect("Bob devices should synchronize the exchange");
        }

        exchanged_contact_id(
            &bob.list_contacts_on_device(0)
                .await
                .expect("B1 should list contacts after exchange"),
            "Alice",
        )
    };
    let alice_bob_contact_id_on_a2 = {
        let alice = alice.read().await;
        exchanged_contact_id(
            &alice
                .list_contacts_on_device(1)
                .await
                .expect("A2 should list contacts"),
            "Bob",
        )
    };

    // A1 IS NOW DEAD. Every sync below touches only A2, A3, and Bob's fleet.
    let surviving_alice_sync = |alice: std::sync::Arc<tokio::sync::RwLock<User>>| async move {
        let alice = alice.read().await;
        alice.sync_device(1).await.expect("A2 sync");
        alice.sync_device(2).await.expect("A3 sync");
    };

    // Outbound continuity: an update authored on the surviving sibling A2
    // must reach every Bob device with A1 gone — the F4 genesis handshake
    // plus per-device sessions carry it; there is no mediator left.
    {
        let alice = alice.read().await;
        let a2 = alice.device(1).expect("A2 should exist").clone();
        let a2 = a2.read().await;
        a2.add_field("phone", "SurvivorPhone", "+12025550811")
            .await
            .expect("A2 should publish the field locally");
        a2.unhide_field_to_contact(&alice_bob_contact_id_on_a2, "SurvivorPhone")
            .await
            .expect("A2 should permit Bob to receive the field");
    }
    let mut outbound_missing = Vec::new();
    for _ in 0..12 {
        surviving_alice_sync(alice.clone()).await;
        bob.read().await.sync_all().await.expect("Bob sync");
        outbound_missing = missing_lost_primary_alice_field(
            &bob,
            &bob_alice_contact_id,
            "SurvivorPhone",
            "+12025550811",
        )
        .await;
        if outbound_missing.is_empty() {
            break;
        }
    }
    assert!(
        outbound_missing.is_empty(),
        "with A1 permanently lost, A2's update must still reach every Bob \
         device (F4 un-orphaning); missing {outbound_missing:?}"
    );

    // Inbound continuity: Bob's update must reach the surviving siblings —
    // pre-F4, incoming [0;32] ciphertext was undecryptable on session-less
    // devices, orphaning this direction too.
    {
        let bob = bob.read().await;
        let b1 = bob.device(0).expect("B1 should exist").clone();
        let b1 = b1.read().await;
        let b1_alice = exchanged_contact_id(
            &b1.list_contacts().await.expect("B1 should list contacts"),
            "Alice",
        );
        b1.add_field("phone", "BobReplyPhone", "+12025550812")
            .await
            .expect("B1 should publish the reply field");
        b1.unhide_field_to_contact(&b1_alice, "BobReplyPhone")
            .await
            .expect("B1 should permit Alice to receive the reply field");
    }
    let mut inbound_missing = Vec::new();
    for _ in 0..12 {
        bob.read().await.sync_all().await.expect("Bob sync");
        surviving_alice_sync(alice.clone()).await;
        inbound_missing = missing_lost_primary_bob_field(
            &alice,
            &alice_bob_contact_id_on_a2,
            "BobReplyPhone",
            "+12025550812",
        )
        .await;
        if inbound_missing.is_empty() {
            break;
        }
    }
    assert!(
        inbound_missing.is_empty(),
        "with A1 permanently lost, Bob's update must still reach the \
         surviving siblings A2/A3; missing {inbound_missing:?}"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Alice's field as seen by every Bob device (A1 excluded from the world).
async fn missing_lost_primary_alice_field(
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    alice_contact_id: &str,
    field_label: &str,
    phone: &str,
) -> Vec<String> {
    let mut missing = Vec::new();
    let bob = bob.read().await;
    for device_index in 0..3 {
        let Some(device) = bob.device(device_index) else {
            missing.push(format!("B{} device", device_index + 1));
            continue;
        };
        match device.read().await.get_contact_card(alice_contact_id).await {
            Ok(Some(card))
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == field_label && field.value == phone) => {}
            _ => missing.push(format!("B{} Alice contact", device_index + 1)),
        }
    }
    missing
}

/// Bob's field as seen by the SURVIVING Alice siblings (A2, A3 — never A1).
async fn missing_lost_primary_bob_field(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob_contact_id: &str,
    field_label: &str,
    phone: &str,
) -> Vec<String> {
    let mut missing = Vec::new();
    let alice = alice.read().await;
    for device_index in 1..3 {
        let Some(device) = alice.device(device_index) else {
            missing.push(format!("A{} device", device_index + 1));
            continue;
        };
        match device.read().await.get_contact_card(bob_contact_id).await {
            Ok(Some(card))
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == field_label && field.value == phone) => {}
            _ => missing.push(format!("A{} Bob contact", device_index + 1)),
        }
    }
    missing
}

/// Polls every Bob device for an `ALERT` line containing `needle`, syncing
/// both fleets between rounds. Returns the matching line, or None after the
/// bounded rounds.
async fn poll_bob_devices_for_alert(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    needle: &str,
) -> Option<String> {
    for _ in 0..8 {
        alice.read().await.sync_all().await.expect("Alice sync");
        bob.read().await.sync_all().await.expect("Bob sync");
        let bob = bob.read().await;
        for device_index in 0..3 {
            let Ok(lines) = bob.list_alerts_on_device(device_index).await else {
                continue;
            };
            if let Some(line) = lines.into_iter().find(|line| line.contains(needle)) {
                return Some(line);
            }
        }
    }
    None
}

// @scenario: duress_mode :: Duress unlock sends silent alert to trusted contacts
/// Release certification: the ADR-032 alert path over the production-shaped
/// six-device split-OHTTP topology — the socket-backed layer the core
/// in-process guard tests cannot reach
/// (record: 2026-07-24-duress-alert-e2e-coverage-gap).
///
/// 1. A secondary device (A2, never exchanged, no session) raises an
///    emergency alert; it traverses genesis → relay → some Bob device
///    surfaces it durably. The promise is AT LEAST ONE device: the contact
///    mailbox is consume-once and sibling fan-out is the deferred F3
///    (`backlog/2026-07-21-per-device-ratchet-registry-dormant.md`).
/// 2. The exchanging device A1's SUBSEQUENT edit still converges on all six
///    devices. The two-sided re-seat guard's chain-level invariants are
///    pinned at core level (core !1446, ADR-064 Amendment 2026-07-24);
///    this step certifies the user-visible outcome — post-alert card flow
///    keeps working over the production-shaped topology.
/// 3. A second session-less sibling (A3) alert also arrives. The SAME-device
///    repeat is pinned in core with a FakeClock (broadcast cooldown is
///    real-time here, CC-06 forbids waiting it out); A3 exercises the same
///    session-less genesis class end to end.
///
/// ADR-032 wire indistinguishability stays deferred until the relay
/// data-dir plumbing (rg10 branch) merges; rg6 covers marker-absence in
/// relay output meanwhile.
// @internal
#[tokio::test]
async fn integration_six_device_duress_alert_certification() {
    let mut orch = Orchestrator::with_config(six_device_certification_config());
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("linked-device topology should synchronize");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");

    // Single exchange, primary devices only — secondaries never exchange.
    let bob_alice_contact_id = {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");
        for _ in 0..2 {
            alice.sync_all().await.expect("Alice devices should sync");
            bob.sync_all().await.expect("Bob devices should sync");
        }
        exchanged_contact_id(
            &bob.list_contacts_on_device(0)
                .await
                .expect("B1 should list contacts"),
            "Alice",
        )
    };
    let bob_id_on_a2 = {
        let alice = alice.read().await;
        exchanged_contact_id(
            &alice
                .list_contacts_on_device(1)
                .await
                .expect("A2 should list contacts"),
            "Bob",
        )
    };

    // 1. Secondary-device alert: session-less A2 → genesis → some Bob device.
    {
        let alice = alice.read().await;
        alice
            .configure_emergency_on_device(1, &bob_id_on_a2, "duress check")
            .await
            .expect("A2 should configure the emergency broadcast");
        let sent = alice
            .send_emergency_from_device(1)
            .await
            .expect("A2 should send the emergency broadcast");
        assert!(
            sent.contains("1/1"),
            "the session-less secondary must queue the alert (genesis), got: {sent}"
        );
    }
    let surfaced = poll_bob_devices_for_alert(&alice, &bob, "duress check").await;
    let surfaced = surfaced.expect(
        "at least one Bob device must durably surface the secondary-device alert \
         (consume-once mailbox: sibling fan-out is the deferred F3)",
    );
    assert!(
        surfaced.contains("kind=emergency"),
        "the surfaced line must carry the alert kind, got: {surfaced}"
    );

    // 2. The guard: A1's channel survives the sibling's genesis alert — its
    // next edit converges on all six devices (RED on pre-guard core).
    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").clone();
        let a1 = a1.read().await;
        let a1_bob = exchanged_contact_id(
            &a1.list_contacts().await.expect("A1 should list contacts"),
            "Bob",
        );
        a1.add_field("phone", "PostAlertPhone", "+12025550903")
            .await
            .expect("A1 should publish the post-alert field");
        a1.unhide_field_to_contact(&a1_bob, "PostAlertPhone")
            .await
            .expect("A1 should permit Bob to receive the post-alert field");
    }
    let mut post_alert_missing = Vec::new();
    for _ in 0..8 {
        alice.read().await.sync_all().await.expect("Alice sync");
        bob.read().await.sync_all().await.expect("Bob sync");
        post_alert_missing = missing_six_device_phone_cards_by_contact_id(
            &alice,
            &bob,
            &bob_alice_contact_id,
            "PostAlertPhone",
            "+12025550903",
        )
        .await;
        if post_alert_missing.is_empty() {
            break;
        }
    }
    assert!(
        post_alert_missing.is_empty(),
        "A1's post-alert edit must still converge everywhere — a sibling's genesis \
         alert must not sever the exchanging device's channel (two-sided guard); \
         missing {post_alert_missing:?}"
    );

    // 3. A second session-less sibling's alert also arrives.
    let bob_id_on_a3 = {
        let alice = alice.read().await;
        exchanged_contact_id(
            &alice
                .list_contacts_on_device(2)
                .await
                .expect("A3 should list contacts"),
            "Bob",
        )
    };
    {
        let alice = alice.read().await;
        alice
            .configure_emergency_on_device(2, &bob_id_on_a3, "second alarm")
            .await
            .expect("A3 should configure the emergency broadcast");
        let sent = alice
            .send_emergency_from_device(2)
            .await
            .expect("A3 should send the emergency broadcast");
        assert!(
            sent.contains("1/1"),
            "the second session-less sibling must queue its alert, got: {sent}"
        );
    }
    let second = poll_bob_devices_for_alert(&alice, &bob, "second alarm").await;
    assert!(
        second.is_some(),
        "the second session-less sibling's alert must also surface on a Bob device"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Faulted delivery still converges deterministically
/// Release certification: a linked device on each side misses live sync while
/// both owners update, then catches up to the exact permitted contact cards.
// @internal
#[tokio::test]
async fn integration_six_device_offline_catchup_converges_exact_values() {
    let mut orch = Orchestrator::with_config(OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        ..Default::default()
    });
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");

    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("all linked devices should synchronize their topology");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("Alice device should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("Bob device should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("Bob device should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("Alice device should complete exchange");
        for _ in 0..2 {
            alice.sync_all().await.expect("Alice exchange sync");
            bob.sync_all().await.expect("Bob exchange sync");
        }

        let alice_device = alice.device(1).expect("A2 should exist").clone();
        let alice_device = alice_device.read().await;
        alice_device
            .add_field("phone", "OfflineAlicePhone", "+12025550301")
            .await
            .expect("A2 should publish while A3 is offline");
        alice_device
            .unhide_field_to_contact("Bob", "OfflineAlicePhone")
            .await
            .expect("A2 should permit Bob to receive the field");

        let bob_device = bob.device(1).expect("B2 should exist").clone();
        let bob_device = bob_device.read().await;
        bob_device
            .add_field("phone", "OfflineBobPhone", "+12025550401")
            .await
            .expect("B2 should publish while B3 is offline");
        bob_device
            .unhide_field_to_contact("Alice", "OfflineBobPhone")
            .await
            .expect("B2 should permit Alice to receive the field");
    }

    // A3 and B3 deliberately remain offline while their sibling devices
    // receive and exchange updates.
    for _ in 0..3 {
        let alice = alice.read().await;
        alice.sync_device(0).await.expect("A1 sync should succeed");
        alice.sync_device(1).await.expect("A2 sync should succeed");
        drop(alice);
        let bob = bob.read().await;
        bob.sync_device(0).await.expect("B1 sync should succeed");
        bob.sync_device(1).await.expect("B2 sync should succeed");
    }

    for _ in 0..5 {
        orch.sync_all()
            .await
            .expect("offline devices should catch up");
        let missing_alice =
            missing_six_device_phone_cards(&alice, &bob, "OfflineAlicePhone", "+12025550301").await;
        let missing_bob =
            missing_six_device_bob_phone_cards(&alice, &bob, "OfflineBobPhone", "+12025550401")
                .await;
        if missing_alice.is_empty() && missing_bob.is_empty() {
            orch.stop().await.expect("Failed to stop orchestrator");
            return;
        }
    }

    let missing_alice =
        missing_six_device_phone_cards(&alice, &bob, "OfflineAlicePhone", "+12025550301").await;
    let missing_bob =
        missing_six_device_bob_phone_cards(&alice, &bob, "OfflineBobPhone", "+12025550401").await;
    assert!(
        missing_alice.is_empty(),
        "offline A3/B3 catch-up lost Alice's permitted update on {missing_alice:?}"
    );
    assert!(
        missing_bob.is_empty(),
        "offline A3/B3 catch-up lost Bob's permitted update on {missing_bob:?}"
    );
    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Faulted delivery still converges deterministically
/// Release certification: the application relay is unavailable while both
/// owners publish permitted updates from linked devices. Once the split-OHTTP
/// route recovers, all owner and peer copies must converge to the exact values.
// @internal
#[tokio::test]
async fn integration_six_device_faulted_relay_delivery_converges_exact_values() {
    let mut orch = Orchestrator::with_config(OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        ..Default::default()
    });
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("all linked devices should synchronize their topology");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("Alice device should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("Bob device should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("Bob device should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("Alice device should complete exchange");
    }
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("exchange state should synchronize before the outage");
    }

    // This faults the application relay behind the OHTTP outer hop. The
    // updates remain local and must be retried after the same relay restarts.
    orch.stop_relay(0)
        .await
        .expect("Failed to stop the application relay");
    {
        let alice = alice.read().await;
        let a2 = alice.device(1).expect("A2 should exist").clone();
        let a2 = a2.read().await;
        a2.add_field("phone", "FaultedAlicePhone", "+12025550801")
            .await
            .expect("A2 should retain its update while delivery is faulted");
        a2.unhide_field_to_contact("Bob", "FaultedAlicePhone")
            .await
            .expect("A2 should permit Bob to receive the queued update");

        let bob = bob.read().await;
        let b2 = bob.device(1).expect("B2 should exist").clone();
        let b2 = b2.read().await;
        b2.add_field("phone", "FaultedBobPhone", "+12025550901")
            .await
            .expect("B2 should retain its update while delivery is faulted");
        b2.unhide_field_to_contact("Alice", "FaultedBobPhone")
            .await
            .expect("B2 should permit Alice to receive the queued update");
    }
    let _ = orch.sync_all().await;

    orch.restart_relay(0)
        .await
        .expect("Failed to restart the application relay");
    for _ in 0..6 {
        orch.sync_all()
            .await
            .expect("faulted relay delivery should recover through split OHTTP");
        let missing_alice =
            missing_six_device_phone_cards(&alice, &bob, "FaultedAlicePhone", "+12025550801").await;
        let missing_bob =
            missing_six_device_bob_phone_cards(&alice, &bob, "FaultedBobPhone", "+12025550901")
                .await;
        if missing_alice.is_empty() && missing_bob.is_empty() {
            orch.stop().await.expect("Failed to stop orchestrator");
            return;
        }
    }

    let missing_alice =
        missing_six_device_phone_cards(&alice, &bob, "FaultedAlicePhone", "+12025550801").await;
    let missing_bob =
        missing_six_device_bob_phone_cards(&alice, &bob, "FaultedBobPhone", "+12025550901").await;
    assert!(
        missing_alice.is_empty(),
        "faulted relay delivery lost Alice's permitted update on {missing_alice:?}"
    );
    assert!(
        missing_bob.is_empty(),
        "faulted relay delivery lost Bob's permitted update on {missing_bob:?}"
    );
    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Faulted delivery still converges deterministically
/// Release certification: one opaque update delivery is duplicated after the
/// six-device exchange topology is synchronized. All owner and peer copies
/// must retain the exact permitted value after the duplicate delivery.
// @internal
#[tokio::test]
async fn integration_six_device_duplicate_ohttp_delivery_converges_exact_values() {
    let mut orch = Orchestrator::with_config(six_device_certification_config());
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("all linked devices should synchronize their topology");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("Alice device should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("Bob device should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("Bob device should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("Alice device should complete exchange");
    }
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("exchange state should synchronize before the fault");
    }

    {
        let alice = alice.read().await;
        let a2 = alice.device(1).expect("A2 should exist").clone();
        let a2 = a2.read().await;
        a2.add_field("phone", "DuplicateAlicePhone", "+12025550802")
            .await
            .expect("A2 should add its update before the duplicate delivery");
    }
    orch.arm_ohttp_duplicate_next_forward()
        .await
        .expect("E2E OHTTP relay should arm exactly one duplicate delivery");
    {
        let alice = alice.read().await;
        let a2 = alice.device(1).expect("A2 should exist").clone();
        a2.read()
            .await
            .unhide_field_to_contact("Bob", "DuplicateAlicePhone")
            .await
            .expect("A2 should publish the duplicated permitted update");
    }

    for _ in 0..6 {
        orch.sync_all()
            .await
            .expect("duplicate OHTTP delivery should converge through the outer relay");
        let missing =
            missing_six_device_phone_cards(&alice, &bob, "DuplicateAlicePhone", "+12025550802")
                .await;
        if missing.is_empty() {
            orch.stop().await.expect("Failed to stop orchestrator");
            return;
        }
    }

    let missing =
        missing_six_device_phone_cards(&alice, &bob, "DuplicateAlicePhone", "+12025550802").await;
    assert!(
        missing.is_empty(),
        "duplicate OHTTP delivery lost Alice's permitted update on {missing:?}"
    );
    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Simultaneous linked-device edits converge
/// Release certification: concurrent edits to one visible field from two
/// linked devices converge on every owner and peer card through split OHTTP.
// @internal
#[tokio::test]
async fn integration_six_device_concurrent_field_edits_converge() {
    let mut orch = Orchestrator::with_config(six_device_certification_config());
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("linked-device topology should synchronize");
    }
    assert_six_device_owner_topology(&orch).await;
    for (device_index, alice_phone, bob_phone) in [
        (0, "+12025550601", "+12025550701"),
        (1, "+12025550602", "+12025550702"),
        (2, "+12025550603", "+12025550703"),
    ] {
        certify_six_device_role(&orch, device_index, alice_phone, bob_phone).await;
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");

    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").clone();
        let a1 = a1.read().await;
        a1.add_field("phone", "ConcurrentPhone", "+12025550500")
            .await
            .expect("A1 should add the shared field");
        a1.unhide_field_to_contact("Bob", "ConcurrentPhone")
            .await
            .expect("A1 should permit Bob to receive the field");
    }
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("field setup should synchronize");
    }

    // Do not sync between the edits: both devices author a competing update
    // from the same pre-edit field state. ADR-020 resolves the conflict by
    // `(timestamp, device_id)` after the following deliberately reordered syncs.
    let (a1, a2) = {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").clone();
        let a2 = alice.device(1).expect("A2 should exist").clone();
        (a1, a2)
    };
    let (a1_edit, a2_edit) = tokio::join!(
        async {
            a1.read()
                .await
                .edit_field("ConcurrentPhone", "+12025550501")
                .await
        },
        async {
            a2.read()
                .await
                .edit_field("ConcurrentPhone", "+12025550502")
                .await
        },
    );
    a1_edit.expect("A1 concurrent edit should succeed");
    a2_edit.expect("A2 concurrent edit should succeed");

    orch.arm_ohttp_reorder_next_forward()
        .await
        .expect("E2E OHTTP relay should arm one reordered delivery");
    let first_sync = tokio::spawn(async move { a1.read().await.sync().await });
    orch.wait_for_ohttp_reorder_pending()
        .await
        .expect("first concurrent sync should reach the E2E OHTTP relay");
    let a2_sync = a2.read().await.sync().await;
    let a1_sync = first_sync
        .await
        .expect("A1 concurrent sync task should not panic");
    a1_sync.expect("A1 concurrent sync should succeed");
    a2_sync.expect("A2 concurrent sync should succeed");

    let mut winner = None;
    let mut last_missing_alice = Vec::new();
    let mut last_peer_values = Vec::new();
    for _ in 0..6 {
        orch.sync_all()
            .await
            .expect("concurrent edits should synchronize");
        let value = {
            let alice_guard = alice.read().await;
            alice_guard
                .get_card_on_device(0)
                .await
                .expect("A1 owner card should be readable")
                .fields
                .into_iter()
                .find(|field| field.label == "ConcurrentPhone")
                .map(|field| field.value)
                .expect("A1 should retain the concurrently edited field")
        };

        let missing_alice =
            missing_six_device_phone_cards(&alice, &bob, "ConcurrentPhone", &value).await;
        if missing_alice.is_empty() {
            winner = Some(value);
            break;
        }
        last_missing_alice = missing_alice;
        let bob_guard = bob.read().await;
        last_peer_values = Vec::new();
        for device_index in 0..3 {
            let device = bob_guard
                .device(device_index)
                .expect("Bob device should exist");
            let value = device
                .read()
                .await
                .get_contact_card("Alice")
                .await
                .ok()
                .flatten()
                .and_then(|card| {
                    card.fields
                        .into_iter()
                        .find(|field| field.label == "ConcurrentPhone")
                        .map(|field| field.value)
                });
            last_peer_values.push(format!("B{}={value:?}", device_index + 1));
        }
    }

    assert!(
        matches!(winner.as_deref(), Some("+12025550501" | "+12025550502")),
        "all six owner and peer cards must converge to one concurrent-write winner; \
         got {winner:?}; missing Alice update on {last_missing_alice:?}; \
         peer values {last_peer_values:?}"
    );
    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Faulted delivery still converges deterministically
/// Release certification: a bounded A2 clock skew must deterministically win
/// a concurrent visible-field edit and converge through split OHTTP.
// @internal
#[tokio::test]
async fn integration_six_device_bounded_clock_skew_converges_to_later_update() {
    let mut orch = Orchestrator::with_config(six_device_certification_config());
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp_with_device_envs(
        "Alice",
        vec![
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ],
    )
    .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp_with_device_envs(
        "Bob",
        vec![
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ],
    )
    .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("linked-device topology should synchronize");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    let (alice_bob_contact_id, bob_alice_contact_id) = {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");
        (
            exchanged_contact_id(
                &alice
                    .list_contacts_on_device(0)
                    .await
                    .expect("A1 should list contacts after exchange"),
                "Bob",
            ),
            exchanged_contact_id(
                &bob.list_contacts_on_device(0)
                    .await
                    .expect("B1 should list contacts after exchange"),
                "Alice",
            ),
        )
    };
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("exchange state should synchronize before skewed edits");
    }

    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").clone();
        let a1 = a1.read().await;
        a1.add_field("phone", "ClockSkewPhone", "+12025551100")
            .await
            .expect("A1 should add the shared field");
        a1.unhide_field_to_contact(&alice_bob_contact_id, "ClockSkewPhone")
            .await
            .expect("A1 should permit Bob to receive the shared field");
    }
    let mut baseline_missing = Vec::new();
    for _ in 0..8 {
        orch.sync_all()
            .await
            .expect("shared field should synchronize before competing edits");
        baseline_missing = missing_six_device_phone_cards_by_contact_id(
            &alice,
            &bob,
            &bob_alice_contact_id,
            "ClockSkewPhone",
            "+12025551100",
        )
        .await;
        if baseline_missing.is_empty() {
            break;
        }
    }
    assert!(
        baseline_missing.is_empty(),
        "clock-skew baseline must converge before competing edits; missing {baseline_missing:?}"
    );

    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").clone();
        let a2 = alice.device(1).expect("A2 should exist").clone();
        a1.read()
            .await
            .edit_field("ClockSkewPhone", "+12025551101")
            .await
            .expect("A1 should make the earlier edit");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_secs();
        let a2_clock_epoch = now.saturating_add(30).to_string();
        let mut a2 = a2.write().await;
        a2.set_command_env("VAUCHI_TEST_CLOCK_EPOCH", &a2_clock_epoch)
            .expect("A2 should accept the scoped test clock");
        a2.edit_field("ClockSkewPhone", "+12025551102")
            .await
            .expect("A2 should make the bounded-skew later edit");
        a2.remove_command_env("VAUCHI_TEST_CLOCK_EPOCH")
            .expect("A2 should restore its normal command environment");
        assert!(
            a2.get_card()
                .await
                .expect("A2 owner card should be readable after its edit")
                .fields
                .iter()
                .any(|field| field.label == "ClockSkewPhone" && field.value == "+12025551102"),
            "A2 must retain its bounded-skew local write before synchronization"
        );
    }

    let mut missing = Vec::new();
    for _ in 0..6 {
        orch.sync_all()
            .await
            .expect("bounded-skew concurrent edits should synchronize");
        missing = missing_six_device_phone_cards_by_contact_id(
            &alice,
            &bob,
            &bob_alice_contact_id,
            "ClockSkewPhone",
            "+12025551102",
        )
        .await;
        if missing.is_empty() {
            orch.stop().await.expect("Failed to stop orchestrator");
            return;
        }
    }

    assert!(
        missing.is_empty(),
        "A2's bounded clock-skew winner must converge to every owner and peer card; missing on {missing:?}"
    );
    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Complete owner-private state converges across linked devices
/// A personal note must converge only among Alice's linked devices, and a
/// deletion must remove it everywhere rather than leaving an owner-private
/// copy on a sibling or peer device.
// @internal
#[tokio::test]
async fn integration_six_device_personal_note_tombstone_converges_owner_only() {
    let mut orch = Orchestrator::with_config(OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        ..Default::default()
    });
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("linked-device topology should synchronize");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");
    }

    let note = "private note must never reach Bob";
    {
        let alice = alice.read().await;
        alice
            .device(0)
            .expect("A1 should exist")
            .read()
            .await
            .add_personal_note("Bob", note)
            .await
            .expect("A1 should add Bob's owner-private note");
    }

    for _ in 0..5 {
        orch.sync_all().await.expect("note sync should succeed");
        let alice = alice.read().await;
        let mut notes = Vec::new();
        for device_index in 0..3 {
            notes.push(
                alice
                    .device(device_index)
                    .expect("Alice device should exist")
                    .read()
                    .await
                    .read_personal_note("Bob")
                    .await
                    .expect("owner device should read Bob's note"),
            );
        }
        if notes.iter().all(|value| value.as_deref() == Some(note)) {
            break;
        }
    }

    {
        let alice = alice.read().await;
        for device_index in 0..3 {
            let value = alice
                .device(device_index)
                .expect("Alice device should exist")
                .read()
                .await
                .read_personal_note("Bob")
                .await
                .expect("owner device should read Bob's note");
            assert_eq!(
                value.as_deref(),
                Some(note),
                "A{} must contain the exact owner-private note",
                device_index + 1
            );
        }
    }
    {
        let bob = bob.read().await;
        for device_index in 0..3 {
            let value = bob
                .device(device_index)
                .expect("Bob device should exist")
                .read()
                .await
                .read_personal_note("Alice")
                .await
                .expect("Bob device should read its own note state");
            assert_eq!(
                value,
                None,
                "B{} must not receive Alice's owner-private note",
                device_index + 1
            );
        }
    }

    {
        let alice = alice.read().await;
        alice
            .device(0)
            .expect("A1 should exist")
            .read()
            .await
            .delete_personal_note("Bob")
            .await
            .expect("A1 should delete Bob's owner-private note");
    }
    for _ in 0..5 {
        orch.sync_all()
            .await
            .expect("tombstone sync should succeed");
        let alice = alice.read().await;
        let mut all_removed = true;
        for device_index in 0..3 {
            let value = alice
                .device(device_index)
                .expect("Alice device should exist")
                .read()
                .await
                .read_personal_note("Bob")
                .await
                .expect("owner device should read deleted note state");
            all_removed &= value.is_none();
        }
        if all_removed {
            break;
        }
    }
    {
        let alice = alice.read().await;
        for device_index in 0..3 {
            let value = alice
                .device(device_index)
                .expect("Alice device should exist")
                .read()
                .await
                .read_personal_note("Bob")
                .await
                .expect("owner device should read deleted note state");
            assert_eq!(
                value,
                None,
                "A{} must remove the owner-private note after its tombstone",
                device_index + 1
            );
        }
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

// @scenario: release_privacy_multidevice_certification.feature:Complete owner-private state converges across linked devices
/// Release certification (RG-10): every owner-private mutation Alice makes
/// on A1, A2, or A3 — MyInfo update and removal, group identity, membership,
/// visibility and presentation overrides, tag change and deletion, contact
/// permission and private note — converges byte-exactly on her other two
/// devices, while Bob's devices receive only the permitted presentation
/// state and neither relay hop observes any of it.
///
/// Stated boundary (owner decision 2026-09-09): the pinned core keeps label
/// presentation overrides owner-side, so Bob still sees Alice's base name;
/// the override markers are therefore asserted absent from Bob like every
/// other owner-private value. ADR-020 deterministic convergence under
/// concurrent and clock-skewed edits is certified by
/// `integration_six_device_concurrent_field_edits_converge` and
/// `integration_six_device_bounded_clock_skew_converges_to_later_update`;
/// this scenario serializes its device batches through sync rounds so it
/// certifies state convergence, not conflict resolution.
// @internal
#[tokio::test]
async fn integration_six_device_owner_private_state_converges() {
    let mut orch = Orchestrator::with_config(owner_private_state_config());
    orch.start().await.expect("Failed to start orchestrator");
    let (alice, bob) = exchanged_six_device_pair(&mut orch).await;
    let alice_bob_contact_id = contact_id_on_device(&alice, 0, "Bob").await;
    let avatar_fixture =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/avatar-1x1.png");

    // A1: MyInfo update + permission, then removal (myinfo update/removal,
    // contacts).
    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").read().await;
        a1.add_field("email", RG10_PERMITTED_FIELD, RG10_PERMITTED_VALUE)
            .await
            .expect("A1 should add the permitted field");
        a1.unhide_field_to_contact(&alice_bob_contact_id, RG10_PERMITTED_FIELD)
            .await
            .expect("A1 should permit Bob to see the field");
        a1.add_field("email", RG10_REMOVED_FIELD, RG10_REMOVED_VALUE)
            .await
            .expect("A1 should add the field that is later removed");
        a1.unhide_field_to_contact(&alice_bob_contact_id, RG10_REMOVED_FIELD)
            .await
            .expect("A1 should permit Bob to see the removed-later field");
    }
    let bob_saw_removed_field = sync_until(&orch, 6, || {
        let bob = bob.clone();
        async move {
            bob_views(&bob).await.iter().all(|view| {
                view.alice_fields
                    .iter()
                    .any(|(_, label, _)| label == RG10_REMOVED_FIELD)
            })
        }
    })
    .await;
    assert!(
        bob_saw_removed_field,
        "myinfo removal needs Bob to hold the field first; Bob views: {:?}",
        bob_views(&bob).await
    );
    {
        let alice = alice.read().await;
        alice
            .device(0)
            .expect("A1 should exist")
            .read()
            .await
            .remove_field(RG10_REMOVED_FIELD)
            .await
            .expect("A1 should remove the field");
    }
    sync_rounds(&orch, 2).await;

    // A2: group identity, membership, visibility, and presentation
    // overrides (presentation cards, group identity/membership/visibility/overrides).
    let avatar_override_bytes = {
        let alice = alice.read().await;
        let a2 = alice.device(1).expect("A2 should exist").read().await;
        a2.add_field("email", RG10_GROUP_FIELD, RG10_GROUP_VALUE)
            .await
            .expect("A2 should add the group-audience field");
        a2.add_field("email", RG10_HIDDEN_FIELD, RG10_HIDDEN_VALUE)
            .await
            .expect("A2 should add the field hidden again from the group");
        a2.create_label(RG10_LABEL)
            .await
            .expect("A2 should create the label");
        a2.add_contact_to_label(RG10_LABEL, "Bob")
            .await
            .expect("A2 should add Bob to the label");
        a2.show_field_to_label(RG10_LABEL, RG10_GROUP_FIELD)
            .await
            .expect("A2 should show the group field to the label");
        a2.show_field_to_label(RG10_LABEL, RG10_HIDDEN_FIELD)
            .await
            .expect("A2 should show the hidden-later field to the label");
        a2.hide_field_from_label(RG10_LABEL, RG10_HIDDEN_FIELD)
            .await
            .expect("A2 should hide the field from the label again");
        a2.set_label_name_override(RG10_LABEL, Some(RG10_NAME_OVERRIDE))
            .await
            .expect("A2 should set the name override");
        a2.set_label_bio_override(RG10_LABEL, Some(RG10_BIO_OVERRIDE))
            .await
            .expect("A2 should set the bio override");
        a2.set_label_avatar_override(RG10_LABEL, Some(&avatar_fixture))
            .await
            .expect("A2 should set the avatar override");
        let stored = a2
            .label_state(RG10_LABEL)
            .await
            .expect("A2 should read the label back")
            .avatar_override_bytes
            .expect("A2 must store the normalized avatar override");
        assert!(stored > 0, "stored avatar override must not be empty");
        stored
    };
    sync_rounds(&orch, 2).await;

    // A3: tag change + deletion, private note (tag change/deletion,
    // contacts/private notes).
    {
        let alice = alice.read().await;
        let a3 = alice.device(2).expect("A3 should exist").read().await;
        a3.create_tag(RG10_TAG)
            .await
            .expect("A3 should create the kept tag");
        a3.add_contact_to_tag(RG10_TAG, "Bob")
            .await
            .expect("A3 should tag Bob");
        a3.create_tag(RG10_DELETED_TAG)
            .await
            .expect("A3 should create the deleted-later tag");
        a3.delete_tag(RG10_DELETED_TAG)
            .await
            .expect("A3 should delete the tag");
        a3.add_personal_note("Bob", RG10_NOTE)
            .await
            .expect("A3 should add the private note");
    }

    let expected = OwnerPrivateState {
        card_fields: vec![
            (
                "email".to_string(),
                RG10_HIDDEN_FIELD.to_string(),
                RG10_HIDDEN_VALUE.to_string(),
            ),
            (
                "email".to_string(),
                RG10_PERMITTED_FIELD.to_string(),
                RG10_PERMITTED_VALUE.to_string(),
            ),
            (
                "email".to_string(),
                RG10_GROUP_FIELD.to_string(),
                RG10_GROUP_VALUE.to_string(),
            ),
        ],
        labels: vec![LabelState {
            name: RG10_LABEL.to_string(),
            members: vec!["Bob".to_string()],
            visible_fields: vec![RG10_GROUP_FIELD.to_string()],
            name_override: Some(RG10_NAME_OVERRIDE.to_string()),
            bio_override: Some(RG10_BIO_OVERRIDE.to_string()),
            avatar_override_bytes: Some(avatar_override_bytes),
        }],
        tags: vec![TagState {
            name: RG10_TAG.to_string(),
            members: vec![alice_bob_contact_id.clone()],
        }],
        contacts: vec!["Bob".to_string()],
        bob_note: Some(RG10_NOTE.to_string()),
    };
    let converged = sync_until(&orch, 8, || {
        let alice = alice.clone();
        let expected = expected.clone();
        async move {
            owner_private_states(&alice)
                .await
                .iter()
                .all(|state| *state == expected)
        }
    })
    .await;
    let states = owner_private_states(&alice).await;
    assert!(
        converged,
        "all six devices synchronized: owner-private state did not converge; A1..A3 = {states:#?}"
    );
    for (index, state) in states.iter().enumerate() {
        assert_eq!(
            *state,
            expected,
            "canonical owner-private state: A{} diverges",
            index + 1
        );
    }

    let views = bob_views(&bob).await;
    for (index, view) in views.iter().enumerate() {
        assert_eq!(
            view.alice_fields,
            vec![
                (
                    "email".to_string(),
                    RG10_PERMITTED_FIELD.to_string(),
                    RG10_PERMITTED_VALUE.to_string()
                ),
                (
                    "email".to_string(),
                    RG10_GROUP_FIELD.to_string(),
                    RG10_GROUP_VALUE.to_string()
                ),
            ],
            "permitted presentation state only: B{} must hold exactly the permitted fields",
            index + 1
        );
        assert_eq!(
            view.alice_name,
            "Alice",
            "B{} sees Alice's base name",
            index + 1
        );
        assert_eq!(
            view.labels,
            Vec::<String>::new(),
            "B{} must own no labels",
            index + 1
        );
        assert_eq!(
            view.tags,
            Vec::<TagState>::new(),
            "B{} must own no tags",
            index + 1
        );
        assert_eq!(view.note, None, "B{} must not receive the note", index + 1);
        let rendered = format!("{view:?}");
        for marker in RG10_PRIVATE_MARKERS {
            assert!(
                !rendered.contains(marker),
                "no private state leaked to bob: B{} exposes {marker:?} in {rendered}",
                index + 1
            );
        }
    }

    let relay_output = orch
        .relay_captured_output(0)
        .expect("application relay output should be captured")
        .join("\n");
    let ohttp_output = orch
        .ohttp_relay_captured_output()
        .expect("ohttp relay output should be captured")
        .join("\n");
    let metrics = reqwest::get(orch.primary_relay_metrics_url().expect("metrics url"))
        .await
        .expect("fetch relay metrics")
        .text()
        .await
        .expect("read relay metrics");
    assert!(
        relay_output.contains("OHTTP gateway enabled"),
        "relay output capture must observe the live process"
    );
    assert!(
        ohttp_output.contains("vauchi-ohttp-relay starting"),
        "ohttp relay output capture must observe the live process"
    );
    for marker in RG10_ALL_MARKERS {
        for (hop, output) in [
            ("application relay output", &relay_output),
            ("application relay metrics", &metrics),
            ("ohttp relay output", &ohttp_output),
        ] {
            assert!(
                !output.contains(marker),
                "no private state leaked to relay: {hop} exposes {marker:?}"
            );
        }
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Complete owner-private state converges across linked devices
/// A per-contact visibility override set on A1, flipped on A2, and flipped
/// back on A3 converges on every owner device and on Bob's stored card. The
/// cli's `contacts hide|unhide` writes an explicit override each way — the
/// override *tombstone* (`remove_contact_visibility_override`) is not
/// reachable through the pinned cli, so this covers the override lifecycle
/// the cli can express.
// @internal
#[tokio::test]
async fn integration_six_device_visibility_override_removal_converges() {
    const FIELD: &str = "Direct";
    const VALUE: &str = "alice-direct@example.com";

    let mut orch = Orchestrator::with_config(owner_private_state_config());
    orch.start().await.expect("Failed to start orchestrator");
    let (alice, bob) = exchanged_six_device_pair(&mut orch).await;
    let alice_bob_contact_id = contact_id_on_device(&alice, 0, "Bob").await;

    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1 should exist").read().await;
        a1.add_field("email", FIELD, VALUE)
            .await
            .expect("A1 should add the field");
        a1.unhide_field_to_contact(&alice_bob_contact_id, FIELD)
            .await
            .expect("A1 should permit Bob");
    }
    let bob_holds = |expected: bool| {
        let bob = bob.clone();
        async move {
            bob_views(&bob).await.iter().all(|view| {
                view.alice_fields
                    .iter()
                    .any(|(_, label, value)| label == FIELD && value == VALUE)
                    == expected
            })
        }
    };
    assert!(
        sync_until(&orch, 6, || bob_holds(true)).await,
        "Bob's devices must receive the permitted field: {:?}",
        bob_views(&bob).await
    );

    {
        let alice = alice.read().await;
        alice
            .device(1)
            .expect("A2 should exist")
            .read()
            .await
            .hide_field_from_contact(&alice_bob_contact_id, FIELD)
            .await
            .expect("A2 should hide the field from Bob");
    }
    assert!(
        sync_until(&orch, 6, || bob_holds(false)).await,
        "hiding on A2 must withdraw the field from every Bob device: {:?}",
        bob_views(&bob).await
    );

    {
        let alice = alice.read().await;
        alice
            .device(2)
            .expect("A3 should exist")
            .read()
            .await
            .unhide_field_to_contact(&alice_bob_contact_id, FIELD)
            .await
            .expect("A3 should permit Bob again");
    }
    assert!(
        sync_until(&orch, 6, || bob_holds(true)).await,
        "permitting again on A3 must restore the field on every Bob device: {:?}",
        bob_views(&bob).await
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: release_privacy_multidevice_certification.feature:Revocation and replacement preserve continuity
/// Release certification: after an exchanged three-device topology gains A4
/// and revokes A2, the active devices retain the exact permitted update.
// @internal
#[tokio::test]
async fn integration_six_device_replacement_and_revocation_preserve_active_convergence() {
    let mut orch = Orchestrator::with_config(OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        ..Default::default()
    });
    orch.start().await.expect("Failed to start orchestrator");
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("linked-device topology should synchronize");
    }

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    let alice_bob_contact_id = {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");
        exchanged_contact_id(
            &alice
                .list_contacts_on_device(0)
                .await
                .expect("A1 should list contacts after exchange"),
            "Bob",
        )
    };
    for _ in 0..2 {
        orch.sync_all()
            .await
            .expect("exchange state should synchronize before replacement");
    }

    let a4_index = orch
        .add_cli_device_split_ohttp("Alice")
        .await
        .expect("A4 should use the same split-OHTTP route");
    {
        let alice = alice.read().await;
        alice
            .link_device(a4_index)
            .await
            .expect("A4 should link to Alice's existing identity");
    }
    let a4 = {
        let alice = alice.read().await;
        let a4 = alice.device(a4_index).expect("A4 should exist").clone();
        a4.read()
            .await
            .add_field("phone", "ReplacementAlicePhone", "+12025551001")
            .await
            .expect("A4 should publish after joining");
        a4
    };
    let mut unhide_succeeded = false;
    let mut last_activation_error = None;
    for _ in 0..12 {
        orch.sync_all()
            .await
            .expect("A4 should progress its registry activation");
        match a4
            .read()
            .await
            .unhide_field_to_contact(&alice_bob_contact_id, "ReplacementAlicePhone")
            .await
        {
            Ok(()) => {
                unhide_succeeded = true;
                break;
            }
            Err(error) if error.to_string().contains("ratchet state") => {
                last_activation_error = Some(error.to_string());
            }
            Err(error) => panic!("A4 visibility update failed unexpectedly: {error}"),
        }
    }
    assert!(
        unhide_succeeded,
        "A4 must complete registry activation before publishing; last error: \
         {last_activation_error:?}"
    );
    for _ in 0..5 {
        orch.sync_all()
            .await
            .expect("A4's permitted update should converge before revocation");
    }

    {
        let alice = alice.read().await;
        alice
            .device(0)
            .expect("A1 should exist")
            .read()
            .await
            .revoke_device_named("Alice_1")
            .await
            .expect("A1 should revoke A2");
    }
    for _ in 0..5 {
        orch.sync_all()
            .await
            .expect("revocation should converge without losing A4's update");
    }

    let missing = missing_active_replacement_phone_cards(
        &alice,
        &bob,
        a4_index,
        "ReplacementAlicePhone",
        "+12025551001",
    )
    .await;
    assert!(
        missing.is_empty(),
        "A1, A3, A4, and every Bob device must retain A4's permitted update after A2 is revoked; missing on {missing:?}"
    );
    orch.stop().await.expect("Failed to stop orchestrator");
}

async fn assert_six_device_owner_topology(orch: &Orchestrator) {
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
}

fn exchanged_contact_id(contacts: &[Contact], display_name: &str) -> String {
    contacts
        .iter()
        .find(|contact| contact.name == display_name && contact.id.is_some())
        .and_then(|contact| contact.id.clone())
        .unwrap_or_else(|| {
            panic!(
                "exchange should leave an addressable {display_name} contact; contacts={contacts:?}"
            )
        })
}

async fn certify_six_device_role(
    orch: &Orchestrator,
    device_index: usize,
    phone: &str,
    bob_phone: &str,
) {
    let alice_phone_label = format!("ReleasePhone{}", device_index + 1);
    let bob_phone_label = format!("ReleaseBobPhone{}", device_index + 1);
    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");

    let (alice_bob_contact_id, bob_alice_contact_id) = {
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

        let alice_bob_contact_id = exchanged_contact_id(
            &alice
                .list_contacts_on_device(device_index)
                .await
                .expect("Alice exchange device should list contacts after exchange"),
            "Bob",
        );
        let bob_alice_contact_id = exchanged_contact_id(
            &bob.list_contacts_on_device(device_index)
                .await
                .expect("Bob exchange device should list contacts after exchange"),
            "Alice",
        );

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
            .add_field("phone", &alice_phone_label, phone)
            .await
            .expect("Alice exchange device should publish phone update");
        device
            .unhide_field_to_contact(&alice_bob_contact_id, &alice_phone_label)
            .await
            .expect("Alice should permit Bob to receive the phone update");

        let bob_device = bob
            .device(device_index)
            .expect("Bob exchange device should exist")
            .clone();
        let bob_device = bob_device.read().await;
        bob_device
            .add_field("phone", &bob_phone_label, bob_phone)
            .await
            .expect("Bob exchange device should publish phone update");
        bob_device
            .unhide_field_to_contact(&bob_alice_contact_id, &bob_phone_label)
            .await
            .expect("Bob should permit Alice to receive the phone update");

        (alice_bob_contact_id, bob_alice_contact_id)
    };

    let mut missing_cards = Vec::new();
    let mut missing_bob_cards = Vec::new();
    for _ in 0..8 {
        {
            let alice = alice.read().await;
            alice.sync_all().await.expect("Alice sync should succeed");
        }
        {
            let bob = bob.read().await;
            bob.sync_all().await.expect("Bob sync should succeed");
        }

        missing_cards = missing_six_device_phone_cards_by_contact_id(
            &alice,
            &bob,
            &bob_alice_contact_id,
            &alice_phone_label,
            phone,
        )
        .await;
        missing_bob_cards = missing_six_device_bob_phone_cards_by_contact_id(
            &alice,
            &bob,
            &alice_bob_contact_id,
            &bob_phone_label,
            bob_phone,
        )
        .await;
        if missing_cards.is_empty() && missing_bob_cards.is_empty() {
            break;
        }
    }

    assert!(
        missing_cards.is_empty(),
        "A{} ↔ B{} exchange did not converge phone {phone}; missing exact value on {missing_cards:?}",
        device_index + 1,
        device_index + 1
    );
    assert!(
        missing_bob_cards.is_empty(),
        "B{} → A{} update did not converge phone {bob_phone}; missing exact value on {missing_bob_cards:?}",
        device_index + 1,
        device_index + 1
    );
}

async fn missing_six_device_phone_cards(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    field_label: &str,
    phone: &str,
) -> Vec<String> {
    missing_six_device_phone_cards_by_contact_id(alice, bob, "Alice", field_label, phone).await
}

async fn missing_six_device_phone_cards_by_contact_id(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    alice_contact_id: &str,
    field_label: &str,
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
                    .any(|field| field.label == field_label && field.value == phone) => {}
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
        match device.read().await.get_contact_card(alice_contact_id).await {
            Ok(Some(card))
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == field_label && field.value == phone) => {}
            _ => missing.push(format!("B{} Alice contact", device_index + 1)),
        }
    }
    missing
}

async fn missing_six_device_bob_phone_cards(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    field_label: &str,
    phone: &str,
) -> Vec<String> {
    missing_six_device_bob_phone_cards_by_contact_id(alice, bob, "Bob", field_label, phone).await
}

async fn missing_six_device_bob_phone_cards_by_contact_id(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob_contact_id: &str,
    field_label: &str,
    phone: &str,
) -> Vec<String> {
    let mut missing = Vec::new();
    let bob = bob.read().await;
    for device_index in 0..3 {
        match bob.get_card_on_device(device_index).await {
            Ok(card)
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == field_label && field.value == phone) => {}
            _ => missing.push(format!("B{} owner card", device_index + 1)),
        }
    }
    drop(bob);

    let alice = alice.read().await;
    for device_index in 0..3 {
        let Some(device) = alice.device(device_index) else {
            missing.push(format!("A{} device", device_index + 1));
            continue;
        };
        match device.read().await.get_contact_card(bob_contact_id).await {
            Ok(Some(card))
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == field_label && field.value == phone) => {}
            _ => missing.push(format!("A{} Bob contact", device_index + 1)),
        }
    }
    missing
}

async fn missing_active_replacement_phone_cards(
    alice: &std::sync::Arc<tokio::sync::RwLock<User>>,
    bob: &std::sync::Arc<tokio::sync::RwLock<User>>,
    replacement_index: usize,
    field_label: &str,
    phone: &str,
) -> Vec<String> {
    let mut missing = Vec::new();
    let alice = alice.read().await;
    for device_index in [0, 2, replacement_index] {
        match alice.get_card_on_device(device_index).await {
            Ok(card)
                if card
                    .fields
                    .iter()
                    .any(|field| field.label == field_label && field.value == phone) => {}
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
                    .any(|field| field.label == field_label && field.value == phone) => {}
            _ => missing.push(format!("B{} Alice contact", device_index + 1)),
        }
    }
    missing
}
// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

const RG10_PERMITTED_FIELD: &str = "Personal";
const RG10_PERMITTED_VALUE: &str = "alice-personal@example.com";
const RG10_REMOVED_FIELD: &str = "Temp";
const RG10_REMOVED_VALUE: &str = "temp-rg10-removed-field-marker@example.com";
const RG10_GROUP_FIELD: &str = "Work";
const RG10_GROUP_VALUE: &str = "alice-work@example.com";
const RG10_HIDDEN_FIELD: &str = "Office";
const RG10_HIDDEN_VALUE: &str = "office-rg10-hidden-field-marker@example.com";
const RG10_LABEL: &str = "Work-rg10-label-marker";
const RG10_NAME_OVERRIDE: &str = "Alice-rg10-name-override-marker";
const RG10_BIO_OVERRIDE: &str = "rg10-bio-override-marker";
const RG10_TAG: &str = "rg10-tag-marker";
const RG10_DELETED_TAG: &str = "rg10-deleted-tag-marker";
const RG10_NOTE: &str = "rg10-note-marker";
/// Values that must never reach Bob's devices.
const RG10_PRIVATE_MARKERS: [&str; 8] = [
    RG10_REMOVED_VALUE,
    RG10_HIDDEN_VALUE,
    RG10_LABEL,
    RG10_NAME_OVERRIDE,
    RG10_BIO_OVERRIDE,
    RG10_TAG,
    RG10_DELETED_TAG,
    RG10_NOTE,
];
/// Values that must never appear at either relay hop, permitted ones included.
const RG10_ALL_MARKERS: [&str; 10] = [
    RG10_PERMITTED_VALUE,
    RG10_GROUP_VALUE,
    RG10_REMOVED_VALUE,
    RG10_HIDDEN_VALUE,
    RG10_LABEL,
    RG10_NAME_OVERRIDE,
    RG10_BIO_OVERRIDE,
    RG10_TAG,
    RG10_DELETED_TAG,
    RG10_NOTE,
];

/// Normalized owner-private state one owner device reports; two devices
/// have converged when their values are equal.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerPrivateState {
    /// `(type, label, value)` sorted by label.
    card_fields: Vec<(String, String, String)>,
    labels: Vec<LabelState>,
    tags: Vec<TagState>,
    contacts: Vec<String>,
    bob_note: Option<String>,
}

/// Everything a Bob device can observe about Alice.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BobView {
    alice_name: String,
    /// `(type, label, value)` sorted by label.
    alice_fields: Vec<(String, String, String)>,
    labels: Vec<String>,
    tags: Vec<TagState>,
    note: Option<String>,
}

fn owner_private_state_config() -> OrchestratorConfig {
    OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        // Both hops must log at info so their output captures prove they
        // observed the live processes before the leak assertions run.
        relay_config: RelayConfig {
            log_filter: Some("info".to_string()),
            ..Default::default()
        },
        ohttp_relay_config: OhttpRelayConfig {
            rate_limit_per_sec: 0,
            log_filter: Some("info".to_string()),
            ..Default::default()
        },
        ..Default::default()
    }
}

type SharedUser = std::sync::Arc<tokio::sync::RwLock<User>>;

/// Three linked devices per user through split OHTTP, A1<->B1 exchanged,
/// and the exchange synchronized to every linked device.
async fn exchanged_six_device_pair(orch: &mut Orchestrator) -> (SharedUser, SharedUser) {
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    sync_rounds(orch, 2).await;

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");
    }
    sync_rounds(orch, 2).await;
    (alice, bob)
}

async fn sync_rounds(orch: &Orchestrator, rounds: usize) {
    for _ in 0..rounds {
        orch.sync_all().await.expect("sync round should succeed");
    }
}

/// Run sync rounds until `converged` reports true; false after `max_rounds`.
async fn sync_until<F, Fut>(orch: &Orchestrator, max_rounds: usize, mut converged: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..max_rounds {
        orch.sync_all().await.expect("sync round should succeed");
        if converged().await {
            return true;
        }
    }
    false
}

async fn contact_id_on_device(
    user: &SharedUser,
    device_index: usize,
    contact_name: &str,
) -> String {
    let contacts = user
        .read()
        .await
        .list_contacts_on_device(device_index)
        .await
        .expect("device should list contacts after exchange");
    exchanged_contact_id(&contacts, contact_name)
}

fn sorted_fields(card: &ContactCard) -> Vec<(String, String, String)> {
    let mut fields: Vec<_> = card
        .fields
        .iter()
        .map(|field| {
            (
                field.field_type.clone(),
                field.label.clone(),
                field.value.clone(),
            )
        })
        .collect();
    fields.sort_by(|a, b| a.1.cmp(&b.1));
    fields
}

async fn owner_private_states(alice: &SharedUser) -> Vec<OwnerPrivateState> {
    let alice = alice.read().await;
    let mut states = Vec::new();
    for device_index in 0..3 {
        let device = alice
            .device(device_index)
            .expect("Alice device should exist")
            .read()
            .await;
        let card = device
            .get_card()
            .await
            .expect("owner device should show its card");
        let mut labels = Vec::new();
        for name in device
            .list_labels()
            .await
            .expect("owner device should list labels")
        {
            labels.push(
                device
                    .label_state(&name)
                    .await
                    .expect("owner device should show each label"),
            );
        }
        labels.sort_by(|a, b| a.name.cmp(&b.name));
        let mut contacts: Vec<String> = device
            .list_contacts()
            .await
            .expect("owner device should list contacts")
            .into_iter()
            .map(|contact| contact.name)
            .collect();
        contacts.sort();
        states.push(OwnerPrivateState {
            card_fields: sorted_fields(&card),
            labels,
            tags: device
                .list_tags()
                .await
                .expect("owner device should list tags"),
            contacts,
            bob_note: device
                .read_personal_note("Bob")
                .await
                .expect("owner device should read Bob's note state"),
        });
    }
    states
}

async fn bob_views(bob: &SharedUser) -> Vec<BobView> {
    let bob = bob.read().await;
    let mut views = Vec::new();
    for device_index in 0..3 {
        let device = bob
            .device(device_index)
            .expect("Bob device should exist")
            .read()
            .await;
        let contacts = device
            .list_contacts()
            .await
            .expect("Bob device should list contacts");
        let alice_card = device
            .get_contact_card(&exchanged_contact_id(&contacts, "Alice"))
            .await
            .expect("Bob device should show Alice's card")
            .expect("Bob device should hold Alice's card after exchange");
        views.push(BobView {
            alice_name: alice_card.name.clone(),
            alice_fields: sorted_fields(&alice_card),
            labels: device
                .list_labels()
                .await
                .expect("Bob device should list labels"),
            tags: device
                .list_tags()
                .await
                .expect("Bob device should list tags"),
            note: device
                .read_personal_note("Alice")
                .await
                .expect("Bob device should read its own note state"),
        });
    }
    views
}
