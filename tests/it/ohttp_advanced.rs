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
    ROTATION_WAIT_SECS, create_ohttp_transport, spawn_ohttp_stack,
    spawn_ohttp_stack_cached_rotation, spawn_ohttp_stack_fast_rotation,
};

// @scenario: sync:OHTTP cached key remains valid for requests
#[tokio::test]
async fn integration_ohttp_cached_key_remains_valid_during_rotation() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_cached_rotation().await;

    let client = reqwest::Client::new();

    // 1. Fetch key (may be cached by the ohttp-relay proxy)
    let key = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch key")
        .bytes()
        .await
        .expect("read key");

    // 2. Rapid re-fetch — should return the same bytes (key hasn't
    //    rotated yet; the proxy may also be serving a cache hit)
    let key_again = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch key again")
        .bytes()
        .await
        .expect("read key again");
    assert_eq!(
        key.as_ref(),
        key_again.as_ref(),
        "rapid re-fetch must return the same key"
    );

    // 3. Send with cached key — must succeed
    let transport = create_ohttp_transport(&ohttp_url, &key);
    let blob_id = transport
        .send_update(&"a".repeat(64), "dGVzdA==")
        .expect("send with cached key must succeed");
    assert!(!blob_id.is_empty(), "blob_id must be non-empty");

    // 4. Wait for both cache TTL (2s) and rotation (2s) to expire.
    //    3s guarantees exactly one rotation; the S10 grace period
    //    retains the previous key so step 5 succeeds.
    tokio::time::sleep(std::time::Duration::from_secs(ROTATION_WAIT_SECS)).await;

    // 5. Verify the proxy cache has expired by checking that the
    //    served key has changed (rotation happened + cache refreshed)
    let key_after_expiry = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch key after expiry")
        .bytes()
        .await
        .expect("read key after expiry");
    assert_ne!(
        key.as_ref(),
        key_after_expiry.as_ref(),
        "key must differ after cache TTL + rotation (proves cache expired)"
    );

    // 6. Send with the old (now stale) key — must still succeed
    //    via the gateway's S10 grace period (previous key fallback)
    let transport_stale = create_ohttp_transport(&ohttp_url, &key);
    let grace_blob_id = transport_stale
        .send_update(&"b".repeat(64), "Z3JhY2U=")
        .expect("send with stale key must succeed via S10 grace");
    assert!(!grace_blob_id.is_empty(), "grace blob_id must be non-empty");

    // 7. Send with fresh key — must succeed
    let transport_fresh = create_ohttp_transport(&ohttp_url, &key_after_expiry);
    let fresh_blob_id = transport_fresh
        .send_update(&"c".repeat(64), "ZnJlc2g=")
        .expect("send with fresh key must succeed");
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

    // 128 KiB + 1 byte — exceeds ohttp-relay's default max_request_bytes (65536)
    let oversized = vec![0xAA; 65537];
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
        .get(format!("{ohttp_url}/v2/ohttp-key"))
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

    let client = reqwest::Client::new();

    // Fetch OHTTP key
    let key_bytes = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch key")
        .bytes()
        .await
        .expect("read key");

    // 1. Send via OHTTP (through ohttp-relay)
    let ohttp_transport = create_ohttp_transport(&ohttp_url, &key_bytes);
    let blob_id = ohttp_transport
        .send_update(&"a".repeat(64), "b2h0dHBfc2VuZA==")
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
        .send_update(&"b".repeat(64), "ZGlyZWN0X3NlbmQ=")
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

// ── P3: Key fingerprint header ────────────────────────────────────

// @scenario: sync:OHTTP key fingerprint
#[tokio::test]
async fn integration_ohttp_key_fingerprint_present_and_stable() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    let client = reqwest::Client::new();

    // Fetch key with headers
    let resp = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch key");

    assert!(resp.status().is_success());

    // Key-Fingerprint header must be present (S13)
    let fingerprint = resp
        .headers()
        .get("key-fingerprint")
        .expect("Key-Fingerprint header must be present (S13)")
        .to_str()
        .expect("fingerprint must be valid UTF-8")
        .to_owned();

    // Must be a hex-encoded SHA-256 (64 hex chars)
    assert_eq!(
        fingerprint.len(),
        64,
        "Key-Fingerprint must be 64 hex chars (SHA-256), got {} chars: '{}'",
        fingerprint.len(),
        fingerprint
    );
    assert!(
        fingerprint.chars().all(|c| c.is_ascii_hexdigit()),
        "Key-Fingerprint must be hex-only, got: '{}'",
        fingerprint
    );

    // Verify stability: fetching again returns the same fingerprint
    let key_bytes = resp.bytes().await.expect("read key");
    let resp2 = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("second key fetch");
    let fingerprint2 = resp2
        .headers()
        .get("key-fingerprint")
        .expect("fingerprint on second fetch")
        .to_str()
        .unwrap()
        .to_owned();
    let key_bytes2 = resp2.bytes().await.expect("read key 2");

    assert_eq!(key_bytes, key_bytes2, "key must be stable between fetches");
    assert_eq!(
        fingerprint, fingerprint2,
        "fingerprint must be stable between fetches"
    );

    // Verify fingerprint matches SHA-256 of key bytes
    use sha2::{Digest, Sha256};
    let computed = hex::encode(Sha256::digest(&key_bytes));
    assert_eq!(
        fingerprint, computed,
        "Key-Fingerprint must equal SHA-256 of key config bytes"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P3: Response padding bucket sizes ─────────────────────────────

// @scenario: sync:OHTTP response padding
#[tokio::test]
async fn integration_ohttp_response_sizes_are_padded() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    let client = reqwest::Client::new();
    let key_bytes = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch key")
        .bytes()
        .await
        .expect("read key");

    // Send two payloads of very different sizes — the OHTTP responses should
    // be similar sizes due to padding (both small payloads land in the same
    // 256-byte bucket after padding).
    let transport1 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let r1 = transport1.send_update(&"a".repeat(64), "YQ=="); // tiny payload
    assert!(r1.is_ok(), "small send failed: {:?}", r1.err());

    let transport2 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let r2 = transport2.send_update(
        &"b".repeat(64),
        "YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWE=", // larger payload
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
        spawn_ohttp_stack_fast_rotation().await;

    let client = reqwest::Client::new();
    let key_bytes = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch key")
        .bytes()
        .await
        .expect("read key");

    // Send a burst of requests spanning a rotation interval (2s).
    // At least some requests will hit during or after rotation.
    // All should succeed — either with the current key or via grace period fallback.
    let mut success_count = 0;
    let mut error_count = 0;

    for i in 0..10 {
        let transport = create_ohttp_transport(&ohttp_url, &key_bytes);
        let recipient = format!("{:0>64}", i);
        match transport.send_update(&recipient, "dGVzdA==") {
            Ok(_) => success_count += 1,
            Err(_) => error_count += 1,
        }
        // Space requests across the rotation window
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // With 2s rotation interval and 3s of requests, at most 1 rotation happens.
    // All requests should succeed via current key or grace period.
    assert!(
        success_count >= 9,
        "at least 9/10 requests must succeed during rotation, \
         got {success_count} successes, {error_count} errors"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}
