// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! An observer between the client and the OHTTP relay sees each fetch
//! response's size and nothing else. The gateway pads fetch responses to a
//! fixed ladder, so an empty mailbox, a few small updates and a catch-up page
//! must all come out as one of three sizes (vauchi/private#603, D-4).

use std::sync::Arc;

use axum::{Router, body::Bytes, extract::State, http::StatusCode, routing::post};
use tokio::sync::Mutex;
use vauchi_core::network::HttpTransport;

use crate::ohttp_helpers::{create_ohttp_transport, signed_key_config, spawn_ohttp_stack};

/// The gateway's fetch ladder (owner decision 2026-10-10 on #603).
const FETCH_LADDER: [usize; 3] = [4 * 1024, 32 * 1024, 96 * 1024];

/// RFC 9458 response encapsulation with ChaCha20-Poly1305 (ADR-046): a
/// max(Nn, Nk) = 32-byte response nonce, then the padded plaintext and a
/// 16-byte tag.
const OHTTP_RESPONSE_OVERHEAD: usize = 32 + 16;

#[derive(Clone)]
struct Hop {
    upstream: String,
    observed: Arc<Mutex<Vec<usize>>>,
    client: reqwest::Client,
}

/// A forwarding hop in front of the OHTTP relay that records the length of
/// every encapsulated response body, as a network observer would.
async fn observing_hop(upstream: String) -> (String, Arc<Mutex<Vec<usize>>>) {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let state = Hop {
        upstream,
        observed: observed.clone(),
        client: reqwest::Client::new(),
    };
    let app = Router::new()
        .route("/v2/ohttp", post(forward))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind observing hop");
    let url = format!("http://{}", listener.local_addr().expect("hop addr"));
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("observing hop");
    });
    (url, observed)
}

async fn forward(State(hop): State<Hop>, body: Bytes) -> (StatusCode, Vec<u8>) {
    let response = hop
        .client
        .post(format!("{}/v2/ohttp", hop.upstream))
        .header("content-type", "message/ohttp-req")
        .body(body)
        .send()
        .await
        .expect("forward to the OHTTP relay");
    let status = StatusCode::from_u16(response.status().as_u16()).expect("status");
    let bytes = response
        .bytes()
        .await
        .expect("read the OHTTP response")
        .to_vec();
    hop.observed.lock().await.push(bytes.len());
    (status, bytes)
}

/// Sends `count` updates of `payload_bytes` each to `mailbox`.
async fn fill_mailbox(
    transport: Arc<HttpTransport>,
    mailbox: String,
    count: usize,
    payload_bytes: usize,
) {
    tokio::task::spawn_blocking(move || {
        let payload = "A".repeat(payload_bytes);
        for _ in 0..count {
            transport
                .send_update(&mailbox, &payload, None)
                .expect("send an update");
        }
    })
    .await
    .expect("send task");
}

/// Fetches the mailbox page by page through `fetcher`, acknowledging each
/// page through `acker`, until the relay reports nothing left. Returns the
/// number of blobs drained. Only `fetcher` passes the observing hop, so every
/// size it records is a fetch response.
async fn drain_mailbox(
    fetcher: Arc<HttpTransport>,
    acker: Arc<HttpTransport>,
    mailbox: String,
) -> usize {
    tokio::task::spawn_blocking(move || {
        let mut drained = 0;
        loop {
            let (blobs, truncated) = fetcher
                .fetch_page(std::slice::from_ref(&mailbox))
                .expect("fetch a page");
            for blob in &blobs {
                acker
                    .acknowledge(&mailbox, &blob.blob_id)
                    .expect("acknowledge a blob");
            }
            drained += blobs.len();
            if !truncated {
                return drained;
            }
        }
    })
    .await
    .expect("fetch task")
}

fn on_ladder(size: usize) -> bool {
    FETCH_LADDER
        .iter()
        .any(|rung| size == rung + OHTTP_RESPONSE_OVERHEAD)
}

// @scenario: sealed_mailbox :: Fetch responses come in a few fixed sizes
#[tokio::test(flavor = "multi_thread")]
async fn every_fetch_response_an_observer_sees_is_a_ladder_size() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;
    let key = signed_key_config(&ohttp_url, relay_mgr.relay(0).expect("relay")).await;
    let (hop_url, observed) = observing_hop(ohttp_url.clone()).await;
    let direct = Arc::new(create_ohttp_transport(&ohttp_url, &key));
    let fetcher = Arc::new(create_ohttp_transport(&hop_url, &key));

    let empty = "e".repeat(64);
    let few = "f".repeat(64);
    let catch_up = "c".repeat(64);

    fill_mailbox(direct.clone(), few.clone(), 3, 200).await;
    fill_mailbox(direct.clone(), catch_up.clone(), 12, 9 * 1024).await;

    for (mailbox, expected_blobs) in [(empty, 0), (few, 3), (catch_up, 12)] {
        let drained = drain_mailbox(fetcher.clone(), direct.clone(), mailbox).await;
        assert_eq!(drained, expected_blobs, "blobs drained");
    }
    let fetch_sizes = observed.lock().await.clone();

    let off_ladder: Vec<usize> = fetch_sizes
        .iter()
        .copied()
        .filter(|s| !on_ladder(*s))
        .collect();
    assert!(
        off_ladder.is_empty(),
        "fetch responses off the ladder {FETCH_LADDER:?} (+{OHTTP_RESPONSE_OVERHEAD}): {off_ladder:?}; all fetch sizes: {fetch_sizes:?}"
    );
    let catch_up_pages = fetch_sizes
        .iter()
        .filter(|s| **s == FETCH_LADDER[2] + OHTTP_RESPONSE_OVERHEAD)
        .count();
    assert!(
        catch_up_pages >= 1,
        "the catch-up must reach the top rung; fetch sizes: {fetch_sizes:?}"
    );
    assert_eq!(
        fetch_sizes[0],
        FETCH_LADDER[0] + OHTTP_RESPONSE_OVERHEAD,
        "an empty mailbox answers at the bottom rung"
    );
    assert_eq!(
        fetch_sizes[1], fetch_sizes[0],
        "a few small updates look like an empty mailbox"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}
