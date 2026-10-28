// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A card update made while the contact is offline waits on the relay,
//! unreadable there, and reaches the contact once they sync.

use vauchi_e2e_tests::prelude::*;

const ALICE_EMAIL: &str = "alice.offline-sync@example.com";

async fn relay_metrics(orch: &Orchestrator) -> String {
    reqwest::get(orch.primary_relay_metrics_url().expect("metrics url"))
        .await
        .expect("fetch relay metrics")
        .text()
        .await
        .expect("read relay metrics")
}

fn blobs_stored(metrics: &str) -> f64 {
    metrics
        .lines()
        .find_map(|line| line.strip_prefix("relay_blobs_stored "))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or_else(|| panic!("relay_blobs_stored missing from metrics:\n{metrics}"))
}

// @scenario: sync_updates :: Sync via relay
// @scenario: sync_updates :: Relay stores updates for offline contacts
// @scenario: sync_updates :: Receive update while offline
// @scenario: release_privacy_multidevice_certification :: Exact two-user update propagates through both privacy hops
#[tokio::test]
async fn integration_an_update_made_while_bob_is_offline_waits_on_the_relay_and_reaches_him() {
    let mut orch = Orchestrator::with_config(OrchestratorConfig {
        // Both hops log at info so their captures prove they observed the
        // live processes before the no-plaintext assertions run.
        relay_config: RelayConfig {
            log_filter: Some("info".to_string()),
            ..Default::default()
        },
        ohttp_relay_config: OhttpRelayConfig {
            log_filter: Some("info".to_string()),
            ..Default::default()
        },
        ..Default::default()
    });
    orch.start().await.expect("start orchestrator");
    orch.add_user("Alice", 1).expect("add Alice");
    orch.add_user("Bob", 1).expect("add Bob");
    orch.create_all_identities().await.expect("identities");
    orch.exchange("Alice", "Bob").await.expect("exchange");
    for _ in 0..2 {
        orch.sync_all().await.expect("settling sync round");
    }
    let alice = orch.user("Alice").expect("Alice");
    let bob = orch.user("Bob").expect("Bob");
    let stored_while_both_settled = blobs_stored(&relay_metrics(&orch).await);

    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Email", ALICE_EMAIL)
            .await
            .expect("Alice adds her email");
        alice
            .device(0)
            .expect("Alice's device")
            .read()
            .await
            .unhide_field_to_contact("Bob", "Email")
            .await
            .expect("Alice shows her email to Bob");
        alice
            .sync_all()
            .await
            .expect("Alice syncs while Bob is offline");
    }

    let metrics = relay_metrics(&orch).await;
    assert!(
        blobs_stored(&metrics) > stored_while_both_settled,
        "the relay holds Alice's update for offline Bob:\n{metrics}"
    );
    let relay_output = orch
        .relay_captured_output(0)
        .expect("relay output")
        .join("\n");
    let ohttp_relay_output = orch
        .ohttp_relay_captured_output()
        .expect("the default topology routes through the OHTTP relay")
        .join("\n");
    assert!(
        relay_output.contains("OHTTP gateway enabled"),
        "the relay capture must observe the live process"
    );
    assert!(
        ohttp_relay_output.contains("vauchi-ohttp-relay starting"),
        "the OHTTP relay capture must observe the live process"
    );
    for (hop, seen) in [
        ("relay output", &relay_output),
        ("relay metrics", &metrics),
        ("OHTTP relay output", &ohttp_relay_output),
    ] {
        assert!(!seen.contains(ALICE_EMAIL), "{hop} exposes the plaintext");
    }

    bob.read()
        .await
        .sync_all()
        .await
        .expect("Bob comes online and syncs");

    let card = bob
        .read()
        .await
        .get_contact_card("Alice")
        .await
        .expect("read Alice's card at Bob")
        .expect("Bob has Alice");
    let emails: Vec<&str> = card
        .fields
        .iter()
        .filter(|f| f.field_type == "email")
        .map(|f| f.value.as_str())
        .collect();
    assert_eq!(emails, [ALICE_EMAIL]);

    orch.stop().await.expect("stop orchestrator");
}
