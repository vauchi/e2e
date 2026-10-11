// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OHTTP E2E tests — P2 (error cases, rate limiting, mixed transports)
//! and P3 (key fingerprint, response padding, concurrent rotation).
//!
//! Split from `ohttp_integration.rs` to stay within file-size limits.

#[allow(dead_code)]
use crate::ohttp_helpers;

use vauchi_core::network::{HttpTransport, HttpTransportConfig, ProxyConfig};
use vauchi_e2e_tests::ohttp_relay_manager::{OhttpRelayConfig, OhttpRelayManager};
use vauchi_e2e_tests::relay_manager::{RelayConfig, RelayManager};

use ohttp_helpers::{
    create_ohttp_transport, signed_key_config, signed_ohttp_key, spawn_ohttp_stack,
    spawn_ohttp_stack_with_cache,
};
use vauchi_protocol::ohttp_key::{SignedKeyConfig, window_of};

// @scenario: sync:OHTTP cached key remains valid for requests
#[tokio::test]
async fn integration_ohttp_cached_key_remains_valid_during_rotation() {
    // The outer relay caches the signed record for an hour: past a window
    // boundary it still hands out yesterday's record, whose key the gateway
    // must keep accepting.
    let (mut relay_mgr, mut ohttp_mgr, relay_url, ohttp_url) =
        spawn_ohttp_stack_with_cache(3_600).await;
    let relay = relay_mgr.relay(0).expect("relay");
    let key = signed_key_config(&ohttp_url, relay).await;
    let record_key = |url: String| async move {
        let body = reqwest::Client::new()
            .get(url)
            .send()
            .await
            .expect("fetch the signed record")
            .bytes()
            .await
            .expect("read the signed record");
        SignedKeyConfig::decode(&body)
            .expect("a well-formed signed record")
            .key_config
    };

    relay
        .advance_ohttp_windows(1)
        .await
        .expect("advance one window");

    let cached = record_key(format!("{ohttp_url}/v2/ohttp-key-signed")).await;
    let current = record_key(format!("{relay_url}/v2/ohttp-key-signed")).await;
    assert_eq!(
        cached, key,
        "the outer relay still serves its cached record"
    );
    assert_ne!(current, key, "the gateway has moved to the next window");

    let stale_blob_id = create_ohttp_transport(&ohttp_url, &cached)
        .send_update(&"b".repeat(64), "Z3JhY2U=", None)
        .expect("yesterday's cached key must still be accepted");
    assert!(
        !stale_blob_id.is_empty(),
        "stale-key blob_id must be non-empty"
    );

    let fresh_blob_id = create_ohttp_transport(&ohttp_url, &current)
        .send_update(&"c".repeat(64), "ZnJlc2g=", None)
        .expect("send with the current key must succeed");
    assert!(!fresh_blob_id.is_empty(), "fresh blob_id must be non-empty");

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P2: Malformed / empty / oversized OHTTP blobs ─────────────────

// @scenario: sync:OHTTP malformed blob
#[tokio::test]
async fn integration_ohttp_malformed_blob_returns_400() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    let client = reqwest::Client::new();

    // Malformed: random bytes that aren't a valid OHTTP encapsulation
    let resp = client
        .post(format!("{ohttp_url}/v2/ohttp"))
        .header("Content-Type", "message/ohttp-req")
        .body(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03])
        .send()
        .await
        .expect("send malformed blob");

    // ohttp-relay forwards to gateway, which can't decapsulate → 400 or 502
    assert!(
        resp.status().is_client_error() || resp.status().is_server_error(),
        "malformed blob must not succeed, got {}",
        resp.status()
    );

    // Empty body
    let resp_empty = client
        .post(format!("{ohttp_url}/v2/ohttp"))
        .header("Content-Type", "message/ohttp-req")
        .body(vec![])
        .send()
        .await
        .expect("send empty body");

    assert!(
        resp_empty.status().is_client_error() || resp_empty.status().is_server_error(),
        "empty OHTTP body must not succeed, got {}",
        resp_empty.status()
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// @scenario: sync:OHTTP oversized blob
#[tokio::test]
async fn integration_ohttp_oversized_blob_rejected() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    let client = reqwest::Client::new();

    // 128 KiB + 1 byte — exceeds ohttp-relay's default max_request_bytes (131072)
    let oversized = vec![0xAA; 131073];
    let resp = client
        .post(format!("{ohttp_url}/v2/ohttp"))
        .header("Content-Type", "message/ohttp-req")
        .body(oversized)
        .send()
        .await
        .expect("send oversized blob");

    assert_eq!(
        resp.status().as_u16(),
        413,
        "oversized blob must be rejected with 413 Payload Too Large, got {}",
        resp.status()
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P2: Rate limiting ─────────────────────────────────────────────

// @scenario: sync:OHTTP rate limit
#[tokio::test]
async fn integration_ohttp_relay_rate_limit_returns_429() {
    let relay_config = RelayConfig {
        http_api_enabled: true,
        ohttp_enabled: true,
        ohttp_key_rotation_hours: 1,
        ..Default::default()
    };

    let mut relay_mgr = RelayManager::with_config(relay_config)
        .await
        .expect("relay manager");
    relay_mgr.spawn(1).await.expect("spawn relay");

    let relay_http_url = relay_mgr.relay(0).expect("relay instance").http_url();

    // Spawn ohttp-relay with very low rate limit (3 req/sec)
    let ohttp_config = OhttpRelayConfig {
        rate_limit_per_sec: 3,
        ..Default::default()
    };
    let mut ohttp_mgr = OhttpRelayManager::new(ohttp_config).expect("ohttp relay manager");
    ohttp_mgr
        .spawn(&relay_http_url)
        .await
        .expect("spawn ohttp-relay");
    let ohttp_url = ohttp_mgr.url().expect("ohttp relay url");

    let client = reqwest::Client::new();

    // Send requests up to the burst limit (3) — should all pass (may be 400/502
    // from invalid OHTTP blob, but NOT 429)
    for i in 0..3 {
        let resp = client
            .post(format!("{ohttp_url}/v2/ohttp"))
            .header("Content-Type", "message/ohttp-req")
            .body(vec![0x01])
            .send()
            .await
            .expect("send request");

        assert_ne!(
            resp.status().as_u16(),
            429,
            "request {i} within burst limit should not be rate-limited"
        );
    }

    // Next request should exceed the burst → 429
    let resp = client
        .post(format!("{ohttp_url}/v2/ohttp"))
        .header("Content-Type", "message/ohttp-req")
        .body(vec![0x01])
        .send()
        .await
        .expect("send over-limit request");

    assert_eq!(
        resp.status().as_u16(),
        429,
        "request over burst limit must return 429 Too Many Requests"
    );

    // Health and key endpoints should NOT be rate-limited
    let health_resp = client
        .get(format!("{ohttp_url}/health"))
        .send()
        .await
        .expect("health check");
    assert_eq!(
        health_resp.status().as_u16(),
        200,
        "health endpoint must not be rate-limited"
    );

    let key_resp = client
        .get(format!("{ohttp_url}/v2/ohttp-key-signed"))
        .send()
        .await
        .expect("key fetch");
    assert!(
        key_resp.status().is_success(),
        "key endpoint must not be rate-limited, got {}",
        key_resp.status()
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P2: Mixed transports coexist ──────────────────────────────────

// @scenario: sync:OHTTP mixed transports
#[tokio::test]
async fn integration_ohttp_and_direct_http_coexist() {
    let (mut relay_mgr, mut ohttp_mgr, relay_url, ohttp_url) = spawn_ohttp_stack().await;

    // Fetch OHTTP key
    let key_bytes = signed_key_config(&ohttp_url, relay_mgr.relay(0).expect("relay")).await;

    // 1. Send via OHTTP (through ohttp-relay)
    let ohttp_transport = create_ohttp_transport(&ohttp_url, &key_bytes);
    let blob_id = ohttp_transport
        .send_update(&"a".repeat(64), "b2h0dHBfc2VuZA==", None)
        .expect("send via OHTTP");
    assert!(!blob_id.is_empty());

    // 2. Send via direct HTTP (bypassing ohttp-relay, hitting relay directly)
    let direct_config = HttpTransportConfig {
        relay_url: relay_url.clone(),
        timeout_ms: 10_000,
        proxy: ProxyConfig::None,
        allow_direct: true,
        pinned_certs: vec![],
    };
    let direct_transport = HttpTransport::new(direct_config);
    let direct_blob_id = direct_transport
        .send_update(&"b".repeat(64), "ZGlyZWN0X3NlbmQ=", None)
        .expect("send via direct HTTP");
    assert!(!direct_blob_id.is_empty());

    // 3. Fetch via OHTTP (should see blob from step 1)
    let ohttp_fetch = create_ohttp_transport(&ohttp_url, &key_bytes);
    let blobs = ohttp_fetch
        .fetch(&["a".repeat(64)])
        .expect("fetch via OHTTP");
    assert_eq!(blobs.len(), 1, "OHTTP fetch must find the OHTTP-sent blob");
    assert_eq!(blobs[0].blob_id, blob_id);

    // 4. Fetch via direct HTTP (should see blob from step 2)
    let direct_config2 = HttpTransportConfig {
        relay_url: relay_url.clone(),
        timeout_ms: 10_000,
        proxy: ProxyConfig::None,
        allow_direct: true,
        pinned_certs: vec![],
    };
    let direct_fetch = HttpTransport::new(direct_config2);
    let direct_blobs = direct_fetch
        .fetch(&["b".repeat(64)])
        .expect("fetch via direct HTTP");
    assert_eq!(
        direct_blobs.len(),
        1,
        "direct fetch must find the direct-sent blob"
    );
    assert_eq!(direct_blobs[0].blob_id, direct_blob_id);

    // Both transports coexist on the same relay
    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P3: Signed key record ─────────────────────────────────────────
//
// The unsigned key's Key-Fingerprint header (S13) left with the unsigned
// route (#288 plan 7.5): the anchor's signature chain is what a client
// checks now, and a record is stable for its whole window.

// @scenario: sync:OHTTP signed key record
#[tokio::test]
async fn integration_ohttp_signed_record_stable_within_window() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;
    let relay = relay_mgr.relay(0).expect("relay");

    let client = reqwest::Client::new();
    let mut bodies = Vec::new();
    for _ in 0..2 {
        let resp = client
            .get(format!("{ohttp_url}/v2/ohttp-key-signed"))
            .send()
            .await
            .expect("fetch the signed record");
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/vnd.vauchi.ohttp-key-signed")
        );
        bodies.push(resp.bytes().await.expect("read the signed record"));
    }
    assert_eq!(
        bodies[0], bodies[1],
        "the record must be stable within its window"
    );

    let held = signed_ohttp_key(&ohttp_url, relay).await;
    let record = SignedKeyConfig::decode(&bodies[0]).expect("a well-formed record");
    assert_eq!(
        held.key_config, record.key_config,
        "the accepted key is the key the record carries"
    );
    assert_eq!(held.window, window_of(relay.ohttp_clock()));

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P3: Response padding bucket sizes ─────────────────────────────

// @scenario: sync:OHTTP response padding
#[tokio::test]
async fn integration_ohttp_response_sizes_are_padded() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    let key_bytes = signed_key_config(&ohttp_url, relay_mgr.relay(0).expect("relay")).await;

    // Send two payloads of very different sizes — the OHTTP responses should
    // be similar sizes due to padding (both small payloads land in the same
    // 256-byte bucket after padding).
    let transport1 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let r1 = transport1.send_update(&"a".repeat(64), "YQ==", None); // tiny payload
    assert!(r1.is_ok(), "small send failed: {:?}", r1.err());

    let transport2 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let r2 = transport2.send_update(
        &"b".repeat(64),
        "YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWE=", // larger payload
        None,
    );
    assert!(r2.is_ok(), "larger send failed: {:?}", r2.err());

    // Both succeed — padding is transparent to the client. The relay pads
    // responses before OHTTP encapsulation and core unpads after decapsulation.
    // A deeper verification of bucket sizes lives in relay/src/padding.rs unit tests.

    // Fetch both and verify they're retrievable (padding roundtrip works)
    let fetch1 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let blobs1 = fetch1.fetch(&["a".repeat(64)]).expect("fetch blob 1");
    assert_eq!(blobs1.len(), 1, "must find blob 1");

    let fetch2 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let blobs2 = fetch2.fetch(&["b".repeat(64)]).expect("fetch blob 2");
    assert_eq!(blobs2.len(), 1, "must find blob 2");

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P3: Concurrent key rotation ───────────────────────────────────

// @scenario: sync:OHTTP concurrent rotation
#[tokio::test]
async fn integration_ohttp_requests_during_rotation_succeed() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_with_cache(0).await;

    let key_bytes = signed_key_config(&ohttp_url, relay_mgr.relay(0).expect("relay")).await;

    // Ten requests with one key; the gateway crosses a window boundary
    // after the fifth. Every one must succeed: the key it was fetched with
    // stays held for the next window.
    let mut failures = Vec::new();
    for i in 0..10 {
        if i == 5 {
            relay_mgr
                .relay(0)
                .expect("relay")
                .advance_ohttp_windows(1)
                .await
                .expect("advance one window");
        }
        let transport = create_ohttp_transport(&ohttp_url, &key_bytes);
        if let Err(e) = transport.send_update(&format!("{i:0>64}"), "dGVzdA==", None) {
            failures.push((i, e.to_string()));
        }
    }

    assert_eq!(
        failures,
        Vec::<(usize, String)>::new(),
        "every request across the boundary"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}
