// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The signed OHTTP key record end to end (#288 plan 5.1): the relay signs
//! under a test anchor made by the real ceremony CLI, the outer relay passes
//! the record through, and core's own acceptance rule decides.

use vauchi_core::network::ohttp_key_trust::{HeldOhttpKey, OhttpKeyRejection, accept_signed_key};
use vauchi_protocol::ohttp_key::{SignedKeyConfig, window_of};

use super::ohttp_helpers::{create_ohttp_transport, spawn_ohttp_stack_with_cache};

async fn fetch(url: &str) -> (reqwest::StatusCode, Vec<u8>) {
    let response = reqwest::Client::new().get(url).send().await.expect("fetch");
    let status = response.status();
    (status, response.bytes().await.expect("read body").to_vec())
}

async fn signed_record(ohttp_url: &str) -> SignedKeyConfig {
    let (status, body) = fetch(&format!("{ohttp_url}/v2/ohttp-key-signed")).await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "signed record via the outer relay"
    );
    SignedKeyConfig::decode(&body).expect("a well-formed signed record")
}

// @scenario: release_privacy_multidevice_certification.feature:Neither relay can decrypt or identify application users
#[tokio::test]
async fn the_record_through_the_outer_relay_is_accepted_and_its_key_works() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with_cache(0).await;
    let relay = relay_mgr.relay(0).expect("relay");
    let anchor = relay
        .ohttp_anchor()
        .expect("the harness gave the relay a test anchor");

    let record = signed_record(&ohttp_url).await;

    let held = accept_signed_key(&record, &anchor, relay.ohttp_clock(), None)
        .expect("core accepts the relay's record under its anchor");
    assert_eq!(held.window, window_of(relay.ohttp_clock()));
    let (_, served) = fetch(&format!("{ohttp_url}/v2/ohttp-key")).await;
    assert_eq!(
        held.key_config, served,
        "the signed key is the key the gateway serves"
    );
    let blob_id = create_ohttp_transport(&ohttp_url, &held.key_config)
        .send_update(&"a".repeat(64), "c2lnbmVk", None)
        .expect("a request sealed to the accepted key goes through");
    assert!(!blob_id.is_empty());

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

/// The point of the anchor: a record is only as good as the key a client
/// already trusts, whatever path delivered it.
// @internal
#[tokio::test]
async fn the_record_is_refused_under_any_other_anchor() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with_cache(0).await;
    let relay = relay_mgr.relay(0).expect("relay");
    let mut other = relay.ohttp_anchor().expect("test anchor");
    other[31] ^= 0x01;

    let record = signed_record(&ohttp_url).await;

    assert_eq!(
        accept_signed_key(&record, &other, relay.ohttp_clock(), None),
        Err(OhttpKeyRejection::IntermediateSignature)
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

/// A client holding today's key takes tomorrow's record as an update, and
/// its old key keeps working for the one window the gateway still holds.
// @internal
#[tokio::test]
async fn across_a_window_boundary_the_client_moves_to_the_next_record() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with_cache(0).await;
    let relay = relay_mgr.relay(0).expect("relay");
    let anchor = relay.ohttp_anchor().expect("test anchor");
    let today: HeldOhttpKey = accept_signed_key(
        &signed_record(&ohttp_url).await,
        &anchor,
        relay.ohttp_clock(),
        None,
    )
    .expect("today's record");

    relay
        .advance_ohttp_windows(1)
        .await
        .expect("advance one window");
    let tomorrow = accept_signed_key(
        &signed_record(&ohttp_url).await,
        &anchor,
        relay.ohttp_clock(),
        Some(&today),
    )
    .expect("tomorrow's record replaces today's");

    assert_eq!(tomorrow.window, today.window + 1);
    assert_ne!(tomorrow.key_config, today.key_config);
    let blob_id = create_ohttp_transport(&ohttp_url, &today.key_config)
        .send_update(&"d".repeat(64), "eWVzdGVyZGF5", None)
        .expect("yesterday's key is still held for one window");
    assert!(!blob_id.is_empty());

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

/// A client must never step back: the record it already replaced is
/// refused even though it is validly signed.
// @internal
#[tokio::test]
async fn a_replayed_older_record_is_refused() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with_cache(0).await;
    let relay = relay_mgr.relay(0).expect("relay");
    let anchor = relay.ohttp_anchor().expect("test anchor");
    let old_record = signed_record(&ohttp_url).await;
    relay
        .advance_ohttp_windows(1)
        .await
        .expect("advance one window");
    let tomorrow = accept_signed_key(
        &signed_record(&ohttp_url).await,
        &anchor,
        relay.ohttp_clock(),
        None,
    )
    .expect("tomorrow's record");

    let replayed = accept_signed_key(&old_record, &anchor, relay.ohttp_clock(), Some(&tomorrow));

    assert_eq!(
        replayed,
        Err(OhttpKeyRejection::OlderThanHeld {
            window: tomorrow.window - 1,
            held: tomorrow.window
        })
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}
