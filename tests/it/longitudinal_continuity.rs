// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RG-15 — long-lived contact continuity across Alice A1-A3 and Bob B1-B3.
//!
//! One exchange stays useful through ignore, archive, an offline sibling and
//! repeated detail changes; only a block ends it (ADR-072, ADR-056, #295).
//! Device replacement is certified by
//! `integration_six_device_replacement_and_revocation_preserve_active_convergence`.

use vauchi_e2e_tests::prelude::*;

use super::six_device::{
    SharedUser, contact_id_on_device, exchanged_six_device_pair, sync_rounds, sync_until,
};

const CONVERGENCE_ROUNDS: usize = 6;
const DAY: u64 = 86_400;
const TEST_CLOCK: &str = "VAUCHI_TEST_CLOCK_EPOCH";

/// Bob's and Alice's ids for each other, per device index.
struct Peers {
    bob_on_alice: Vec<String>,
    alice_on_bob: Vec<String>,
}

// @scenario: release_privacy_multidevice_certification.feature:Ignoring a contact removes attention but keeps continuity
// @scenario: release_privacy_multidevice_certification.feature:Only blocking ends long-lived contact continuity
// @internal
#[tokio::test]
async fn integration_six_device_longitudinal_contact_continuity_certification() {
    let mut orch = Orchestrator::with_config(OrchestratorConfig {
        inject_local_ohttp_key_into_cli: false,
        ..Default::default()
    });
    orch.start().await.expect("Failed to start orchestrator");
    let (alice, bob) = exchanged_six_device_pair(&mut orch).await;
    let peers = Peers {
        bob_on_alice: peer_ids(&alice, "Bob").await,
        alice_on_bob: peer_ids(&bob, "Alice").await,
    };
    publish(&alice, 1, "LongAlicePhone", "+12025550601", Some("Bob")).await;
    publish(&bob, 1, "LongBobPhone", "+12025550701", Some("Alice")).await;
    assert_both_directions(&orch, &alice, &bob, &peers, "+12025550601", "+12025550701").await;

    // Each phase runs days later, so every device crosses daily mailbox-token
    // rotations (ADR-029) between phases; continuity must survive them.
    let exchanged_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    set_clock(&alice, &bob, exchanged_at + DAY).await;
    ignore_keeps_continuity(&orch, &alice, &bob, &peers).await;
    set_clock(&alice, &bob, exchanged_at + 8 * DAY).await;
    archive_and_offline_catch_up_keep_continuity(&orch, &alice, &bob, &peers).await;
    set_clock(&alice, &bob, exchanged_at + 40 * DAY).await;
    block_ends_continuity(&orch, &alice, &bob, &peers).await;

    orch.stop().await.expect("Failed to stop orchestrator");
}

async fn ignore_keeps_continuity(
    orch: &Orchestrator,
    alice: &SharedUser,
    bob: &SharedUser,
    peers: &Peers,
) {
    act(
        alice,
        2,
        &peers.bob_on_alice[2],
        ContactLifecycleAction::Ignore,
    )
    .await;
    assert_alice_lifecycle(
        orch,
        alice,
        peers,
        ContactLifecycle {
            ignored: true,
            ..Default::default()
        },
    )
    .await;

    publish(alice, 0, "LongAlicePhone", "+12025550602", None).await;
    publish(bob, 2, "LongBobPhone", "+12025550702", None).await;
    assert_both_directions(orch, alice, bob, peers, "+12025550602", "+12025550702").await;
    assert_eq!(
        lifecycles(bob, &peers.alice_on_bob).await,
        vec![ContactLifecycle::default(); 3],
        "Bob is never told he is ignored"
    );

    act(
        alice,
        0,
        &peers.bob_on_alice[0],
        ContactLifecycleAction::Unignore,
    )
    .await;
    assert_alice_lifecycle(orch, alice, peers, ContactLifecycle::default()).await;
}

async fn archive_and_offline_catch_up_keep_continuity(
    orch: &Orchestrator,
    alice: &SharedUser,
    bob: &SharedUser,
    peers: &Peers,
) {
    act(
        alice,
        1,
        &peers.bob_on_alice[1],
        ContactLifecycleAction::Archive,
    )
    .await;
    assert_alice_lifecycle(
        orch,
        alice,
        peers,
        ContactLifecycle {
            archived: true,
            ..Default::default()
        },
    )
    .await;

    // A3 stays offline while Bob changes his details and Alice unarchives.
    publish(bob, 0, "LongBobPhone", "+12025550703", None).await;
    act(
        alice,
        0,
        &peers.bob_on_alice[0],
        ContactLifecycleAction::Unarchive,
    )
    .await;
    for _ in 0..3 {
        for index in 0..3 {
            bob.read().await.sync_device(index).await.expect("Bob sync");
        }
        alice.read().await.sync_device(0).await.expect("A1 sync");
        alice.read().await.sync_device(1).await.expect("A2 sync");
    }

    assert_alice_lifecycle(orch, alice, peers, ContactLifecycle::default()).await;
    assert!(
        sync_until(orch, CONVERGENCE_ROUNDS, || async {
            holders(alice, &peers.bob_on_alice, "LongBobPhone", "+12025550703").await == [true; 3]
        })
        .await,
        "A3 must catch up on Bob's change made while it was offline and Bob was archived; holders: {:?}",
        holders(alice, &peers.bob_on_alice, "LongBobPhone", "+12025550703").await
    );
}

async fn block_ends_continuity(
    orch: &Orchestrator,
    alice: &SharedUser,
    bob: &SharedUser,
    peers: &Peers,
) {
    act(
        alice,
        0,
        &peers.bob_on_alice[0],
        ContactLifecycleAction::Block,
    )
    .await;
    assert_alice_lifecycle(
        orch,
        alice,
        peers,
        ContactLifecycle {
            blocked: true,
            ..Default::default()
        },
    )
    .await;

    publish(alice, 2, "LongAlicePhone", "+12025550609", None).await;
    publish(bob, 1, "LongBobPhone", "+12025550709", None).await;
    sync_rounds(orch, CONVERGENCE_ROUNDS).await;

    assert_eq!(
        holders(bob, &peers.alice_on_bob, "LongAlicePhone", "+12025550609").await,
        vec![false; 3],
        "no Alice device, A3 included, may send Bob an update after A1 blocked him"
    );
    assert_eq!(
        holders(alice, &peers.bob_on_alice, "LongBobPhone", "+12025550709").await,
        vec![false; 3],
        "no Alice device may accept Bob's update after A1 blocked him"
    );
}

/// Moves every one of the six devices' clocks to `epoch` (the CLI's
/// `e2e-test-clock` build reads it on each command).
async fn set_clock(alice: &SharedUser, bob: &SharedUser, epoch: u64) {
    for user in [alice, bob] {
        let user = user.read().await;
        for index in 0..3 {
            user.device(index)
                .expect("device should exist")
                .write()
                .await
                .set_command_env(TEST_CLOCK, &epoch.to_string())
                .expect("CLI devices accept a test clock");
        }
    }
}

async fn peer_ids(user: &SharedUser, peer_name: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for index in 0..3 {
        ids.push(contact_id_on_device(user, index, peer_name).await);
    }
    ids
}

/// Adds `label` (granting it to `grant_to`) or, when it exists, edits it.
async fn publish(
    user: &SharedUser,
    index: usize,
    label: &str,
    value: &str,
    grant_to: Option<&str>,
) {
    let user = user.read().await;
    let device = user
        .device(index)
        .expect("device should exist")
        .read()
        .await;
    match grant_to {
        Some(peer) => {
            device
                .add_field("phone", label, value)
                .await
                .expect("add field");
            device
                .unhide_field_to_contact(peer, label)
                .await
                .expect("grant field");
        }
        None => device.edit_field(label, value).await.expect("edit field"),
    }
}

async fn act(user: &SharedUser, index: usize, contact: &str, action: ContactLifecycleAction) {
    let user = user.read().await;
    user.device(index)
        .expect("device should exist")
        .read()
        .await
        .apply_contact_lifecycle(contact, action)
        .await
        .unwrap_or_else(|e| panic!("{action:?} on device {index} failed: {e}"));
}

async fn lifecycles(user: &SharedUser, contact_ids: &[String]) -> Vec<ContactLifecycle> {
    let user = user.read().await;
    let mut states = Vec::new();
    for (index, contact) in contact_ids.iter().enumerate() {
        let device = user
            .device(index)
            .expect("device should exist")
            .read()
            .await;
        states.push(device.contact_lifecycle(contact).await.expect("lifecycle"));
    }
    states
}

async fn holders(user: &SharedUser, contact_ids: &[String], label: &str, value: &str) -> Vec<bool> {
    let user = user.read().await;
    let mut held = Vec::new();
    for (index, contact) in contact_ids.iter().enumerate() {
        let device = user
            .device(index)
            .expect("device should exist")
            .read()
            .await;
        let card = device
            .get_contact_card(contact)
            .await
            .expect("contact card");
        held.push(card.is_some_and(|card| {
            card.fields
                .iter()
                .any(|f| f.label == label && f.value == value)
        }));
    }
    held
}

async fn assert_alice_lifecycle(
    orch: &Orchestrator,
    alice: &SharedUser,
    peers: &Peers,
    expected: ContactLifecycle,
) {
    assert!(
        sync_until(orch, CONVERGENCE_ROUNDS, || async {
            lifecycles(alice, &peers.bob_on_alice).await == [expected; 3]
        })
        .await,
        "every Alice device must hold {expected:?} for Bob; got {:?}",
        lifecycles(alice, &peers.bob_on_alice).await
    );
}

async fn assert_both_directions(
    orch: &Orchestrator,
    alice: &SharedUser,
    bob: &SharedUser,
    peers: &Peers,
    alice_value: &str,
    bob_value: &str,
) {
    let converged = sync_until(orch, CONVERGENCE_ROUNDS, || async {
        holders(bob, &peers.alice_on_bob, "LongAlicePhone", alice_value).await == [true; 3]
            && holders(alice, &peers.bob_on_alice, "LongBobPhone", bob_value).await == [true; 3]
    })
    .await;
    assert!(
        converged,
        "updates must keep flowing both ways; Bob holds Alice's {alice_value}: {:?}, Alice holds Bob's {bob_value}: {:?}",
        holders(bob, &peers.alice_on_bob, "LongAlicePhone", alice_value).await,
        holders(alice, &peers.bob_on_alice, "LongBobPhone", bob_value).await
    );
}
