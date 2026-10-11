// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Delayed OHTTP through the real outer relay (private#628, design
//! `2026-10-11-outer-relay-delayed-forwarding-design`, step 5).
//!
//! Core's HTTP adapter posts fetches and acknowledgements to the outer
//! relay's `/v2/ohttp-async` and collects the answers on a later cycle;
//! sends stay synchronous. The outer relay forwards at once here
//! (`forward_delay_max_secs: 0`), so a cycle's answer is waiting for the
//! next one. Each test drives the adapter the way `Vauchi::sync` does: a
//! fresh adapter per cycle over one long-lived `DelayedRequests`.

use std::sync::{Arc, Mutex};

use vauchi_core::network::{
    AckStatus, Acknowledgment, DelayedRequests, HttpTransportAdapter, MessageEnvelope,
    MessagePayload, PROTOCOL_VERSION, RegisterMailbox, Transport,
};
use vauchi_e2e_tests::ohttp_relay_manager::{OhttpRelayConfig, OhttpRelayManager};

use crate::ohttp_helpers::{create_ohttp_transport, signed_key_config, spawn_ohttp_stack_with};

const HELD: &str = "holding ticketed OHTTP request";
const FORWARDED: &str = "forwarding delayed OHTTP request";
const LOST: &str = "outcome=\"unknown\"";

fn debug_logging_outer_relay() -> OhttpRelayConfig {
    OhttpRelayConfig {
        log_filter: Some("vauchi_ohttp_relay=debug".to_string()),
        ..OhttpRelayConfig::default()
    }
}

fn envelope(payload: MessagePayload) -> MessageEnvelope {
    MessageEnvelope {
        version: PROTOCOL_VERSION,
        message_id: "e2e".to_string().into(),
        timestamp: 0,
        payload,
    }
}

/// One sync cycle's adapter over the long-lived pending state.
fn cycle(
    ohttp_url: &str,
    key: &[u8],
    token: &str,
    delayed: &Arc<Mutex<DelayedRequests>>,
) -> HttpTransportAdapter {
    let mut adapter = HttpTransportAdapter::new(create_ohttp_transport(ohttp_url, key))
        .with_delayed(delayed.clone());
    adapter
        .send(&envelope(MessagePayload::RegisterMailbox(
            RegisterMailbox {
                tokens: vec![token.to_string()],
            },
        )))
        .expect("register the mailbox token");
    adapter
}

fn count(ohttp: &OhttpRelayManager, needle: &str) -> usize {
    ohttp
        .instance()
        .expect("outer relay running")
        .captured_output()
        .iter()
        .filter(|line| line.contains(needle))
        .count()
}

// @internal
#[tokio::test]
async fn integration_delayed_fetch_and_ack_go_through_the_outer_relay() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with(debug_logging_outer_relay()).await;
    let key = signed_key_config(&ohttp_url, relay_mgr.relay(0).expect("relay")).await;
    let token = "d".repeat(64);
    let sender = create_ohttp_transport(&ohttp_url, &key);
    let blob_id = sender
        .send_update(&token, "ZGVsYXllZA==", None)
        .expect("a send stays synchronous");
    let delayed = DelayedRequests::shared();

    assert!(
        cycle(&ohttp_url, &key, &token, &delayed)
            .receive()
            .expect("first cycle")
            .is_none(),
        "the first cycle only posts the fetch"
    );
    ohttp_mgr
        .wait_for_output(FORWARDED, 1)
        .await
        .expect("the outer relay forwards the held fetch");

    let mut second = cycle(&ohttp_url, &key, &token, &delayed);
    let mail = second
        .receive()
        .expect("second cycle")
        .expect("the second cycle collects the update");
    assert_eq!(mail.message_id.clone().into_string(), blob_id);

    second
        .send(&envelope(MessagePayload::Acknowledgment(Acknowledgment {
            message_id: mail.message_id,
            status: AckStatus::ReceivedByRecipient,
            error: None,
        })))
        .expect("queue the acknowledgement");
    assert!(second.receive().expect("flush the ack").is_none());
    ohttp_mgr
        .wait_for_output(FORWARDED, 3)
        .await
        .expect("the outer relay forwards the ack and the next fetch");

    let left = sender
        .fetch(std::slice::from_ref(&token))
        .expect("a synchronous fetch");
    assert!(
        left.is_empty(),
        "the delayed acknowledgement removed the update: {left:?}"
    );
    assert!(
        count(&ohttp_mgr, HELD) >= 3,
        "fetch, ack and the next fetch were held for a delayed forward"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// @internal
#[tokio::test]
async fn integration_a_synchronous_client_never_touches_the_delayed_path() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with(debug_logging_outer_relay()).await;
    let key = signed_key_config(&ohttp_url, relay_mgr.relay(0).expect("relay")).await;
    let token = "e".repeat(64);
    let sender = create_ohttp_transport(&ohttp_url, &key);
    let blob_id = sender.send_update(&token, "c3luYw==", None).expect("send");
    let mut adapter = HttpTransportAdapter::new(create_ohttp_transport(&ohttp_url, &key));
    adapter
        .send(&envelope(MessagePayload::RegisterMailbox(
            RegisterMailbox {
                tokens: vec![token.clone()],
            },
        )))
        .expect("register");

    let mail = adapter.receive().expect("receive").expect("mail at once");

    assert_eq!(mail.message_id.clone().into_string(), blob_id);
    assert_eq!(
        count(&ohttp_mgr, HELD),
        0,
        "the control for the delayed tests' log observation"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// @internal
#[tokio::test]
async fn integration_a_delayed_fetch_survives_an_outer_relay_restart() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with(debug_logging_outer_relay()).await;
    let key = signed_key_config(&ohttp_url, relay_mgr.relay(0).expect("relay")).await;
    let token = "f".repeat(64);
    let blob_id = create_ohttp_transport(&ohttp_url, &key)
        .send_update(&token, "cmVzdGFydA==", None)
        .expect("send");
    let delayed = DelayedRequests::shared();
    assert!(
        cycle(&ohttp_url, &key, &token, &delayed)
            .receive()
            .expect("post")
            .is_none()
    );
    ohttp_mgr
        .wait_for_output(FORWARDED, 1)
        .await
        .expect("the answer is held");

    ohttp_mgr.restart().await.expect("restart the outer relay");
    assert!(
        cycle(&ohttp_url, &key, &token, &delayed)
            .receive()
            .expect("the lost answer is not an error")
            .is_none(),
        "the held answer died with the outer relay"
    );
    ohttp_mgr
        .wait_for_output(FORWARDED, 2)
        .await
        .expect("the fetch is posted again and forwarded");
    let mail = cycle(&ohttp_url, &key, &token, &delayed)
        .receive()
        .expect("third cycle")
        .expect("the re-posted fetch delivers the update");

    assert_eq!(mail.message_id.clone().into_string(), blob_id);
    assert_eq!(count(&ohttp_mgr, LOST), 1, "one collect found nothing");

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

/// Two CLI users in delayed mode: each `vauchi sync` waits in-process for
/// the fetch it posted (KU-F7), so an update reaches the other side.
// @internal
#[tokio::test]
async fn integration_cli_users_in_delayed_mode_receive_updates() {
    let mut config = vauchi_e2e_tests::orchestrator::OrchestratorConfig::default();
    config.ohttp_relay_config.log_filter = Some("vauchi_ohttp_relay=debug".to_string());
    let mut orch = vauchi_e2e_tests::orchestrator::Orchestrator::with_config(config);
    orch.start().await.expect("start");
    let delayed_env: std::collections::HashMap<String, String> = [
        ("VAUCHI_OHTTP_DELAYED", "1"),
        ("VAUCHI_OHTTP_DELAYED_POLL_SECS", "1"),
        ("VAUCHI_OHTTP_DELAYED_WAIT_SECS", "20"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value.to_string()))
    .collect();
    for name in ["Alice", "Bob"] {
        orch.add_user_split_ohttp_with_device_envs(name, vec![delayed_env.clone()])
            .expect("delayed user");
    }
    orch.create_all_identities().await.expect("identities");
    orch.exchange("Alice", "Bob").await.expect("exchange");
    let alice = orch.user("Alice").expect("Alice");
    let bob = orch.user("Bob").expect("Bob");
    {
        let alice = alice.read().await;
        let a1 = alice.device(0).expect("A1").clone();
        let a1 = a1.read().await;
        a1.add_field("phone", "DelayedPhone", "+12025550628")
            .await
            .expect("add field");
        a1.unhide_field_to_contact("Bob", "DelayedPhone")
            .await
            .expect("show it to Bob");
    }

    let mut arrived = false;
    for _ in 0..4 {
        alice.read().await.sync_all().await.expect("Alice sync");
        bob.read().await.sync_all().await.expect("Bob sync");
        let card = bob
            .read()
            .await
            .get_contact_card("Alice")
            .await
            .expect("Bob reads Alice's card");
        if card.is_some_and(|card| {
            card.fields
                .iter()
                .any(|field| field.label == "DelayedPhone" && field.value == "+12025550628")
        }) {
            arrived = true;
            break;
        }
    }

    assert!(
        arrived,
        "Alice's update reached Bob through delayed fetches"
    );
    let held = orch
        .ohttp_relay_captured_output()
        .expect("outer relay output")
        .iter()
        .filter(|line| line.contains(HELD))
        .count();
    assert!(held > 0, "the CLI clients fetched through the delayed path");
    orch.stop().await.expect("stop");
}
