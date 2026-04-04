// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OHTTP E2E integration tests.
//!
//! Validates the full OHTTP privacy chain:
//! ```text
//! Client (HttpTransport + OhttpClient)
//!   → ohttp-relay (knows client IP, sees encrypted blob)
//!   → vauchi-relay (knows content, sees ohttp-relay IP)
//! ```
//!
//! These tests exercise the real binaries end-to-end — no mocks.

use vauchi_core::network::{HttpTransport, HttpTransportConfig, OhttpClient, ProxyConfig};
use vauchi_e2e_tests::ohttp_relay_manager::{OhttpRelayConfig, OhttpRelayManager};
use vauchi_e2e_tests::relay_manager::{RelayConfig, RelayManager};

/// Spawn vauchi-relay with OHTTP enabled + ohttp-relay forwarding proxy.
/// Returns (relay_manager, ohttp_relay_manager, relay_http_url, ohttp_relay_url).
async fn spawn_ohttp_stack() -> (RelayManager, OhttpRelayManager, String, String) {
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

    let mut ohttp_mgr =
        OhttpRelayManager::new(OhttpRelayConfig::default()).expect("ohttp relay manager");
    ohttp_mgr
        .spawn(&relay_http_url)
        .await
        .expect("spawn ohttp-relay");

    let ohttp_url = ohttp_mgr.url().expect("ohttp relay url");

    (relay_mgr, ohttp_mgr, relay_http_url, ohttp_url)
}

/// Create an HttpTransport configured to use the ohttp-relay, with OHTTP encryption active.
fn create_ohttp_transport(ohttp_relay_url: &str, ohttp_key: &[u8]) -> HttpTransport {
    let config = HttpTransportConfig {
        relay_url: ohttp_relay_url.to_string(),
        timeout_ms: 10_000,
        proxy: ProxyConfig::None,
        allow_direct: false,
    };
    let mut transport = HttpTransport::new(config);

    let client = OhttpClient::new(ohttp_key.to_vec()).expect("create OhttpClient from key");
    transport.set_ohttp(client);

    transport
}

// ── P1: Key Bootstrap ──────────────────────────────────────────────

// @scenario: sync:OHTTP key fetch
#[tokio::test]
async fn test_ohttp_key_bootstrap_via_relay() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    // Fetch OHTTP key config via ohttp-relay → vauchi-relay
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/v2/ohttp-key", ohttp_url))
        .send()
        .await
        .expect("fetch ohttp key");

    assert!(
        resp.status().is_success(),
        "OHTTP key fetch failed: {}",
        resp.status()
    );

    let key_bytes = resp.bytes().await.expect("read key bytes");
    assert!(!key_bytes.is_empty(), "OHTTP key config must not be empty");

    // Verify the key is usable: create an OhttpClient from it
    let ohttp_client = OhttpClient::new(key_bytes.to_vec());
    assert!(
        ohttp_client.is_ok(),
        "OHTTP key must be a valid config: {:?}",
        ohttp_client.err()
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P1: Send via OHTTP ────────────────────────────────────────────

// @scenario: sync:OHTTP send
#[tokio::test]
async fn test_send_and_fetch_via_ohttp() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    // 1. Fetch OHTTP key
    let client = reqwest::Client::new();
    let key_bytes = client
        .get(format!("{}/v2/ohttp-key", ohttp_url))
        .send()
        .await
        .expect("fetch key")
        .bytes()
        .await
        .expect("read key");

    // 2. Create transport with OHTTP
    let transport = create_ohttp_transport(&ohttp_url, &key_bytes);

    // 3. Health check (uses direct endpoint, not OHTTP)
    // Health check goes to the ohttp-relay which has /health
    // but HttpTransport.health_check() hits relay_url/health
    // ohttp-relay also serves /health, so this should work.

    // 4. Send a message via OHTTP
    let recipient_id = "a".repeat(64); // 64-char hex-like ID
    let result = transport.send_update(&recipient_id, "dGVzdCBwYXlsb2Fk");

    assert!(result.is_ok(), "send via OHTTP failed: {:?}", result.err());

    let blob_id = result.unwrap();
    assert!(!blob_id.is_empty(), "blob_id must not be empty");

    // 5. Fetch via OHTTP — need a fresh transport (OHTTP encapsulation is one-shot)
    let transport2 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let fetched = transport2.fetch(&[recipient_id.clone()]);

    assert!(
        fetched.is_ok(),
        "fetch via OHTTP failed: {:?}",
        fetched.err()
    );

    let blobs = fetched.unwrap();
    assert_eq!(
        blobs.len(),
        1,
        "expected 1 fetched blob, got {}",
        blobs.len()
    );
    assert_eq!(blobs[0].blob_id, blob_id);

    // 6. Ack via OHTTP
    let transport3 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let ack_result = transport3.acknowledge(&recipient_id, &blob_id);
    assert!(
        ack_result.is_ok(),
        "ack via OHTTP failed: {:?}",
        ack_result.err()
    );

    // 7. Verify fetch returns empty after ack
    let transport4 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let fetched_after = transport4.fetch(&[recipient_id]);
    assert!(fetched_after.is_ok());
    assert!(
        fetched_after.unwrap().is_empty(),
        "fetch after ack should return no blobs"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P1: Exchange via OHTTP ─────────────────────────────────────────

// @scenario: exchange:OHTTP relay exchange
#[tokio::test]
async fn test_exchange_offer_claim_complete_via_ohttp() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    // Fetch OHTTP key
    let client = reqwest::Client::new();
    let key_bytes = client
        .get(format!("{}/v2/ohttp-key", ohttp_url))
        .send()
        .await
        .expect("fetch key")
        .bytes()
        .await
        .expect("read key");

    // Initiator: post offer
    let t_offer = create_ohttp_transport(&ohttp_url, &key_bytes);
    let code = t_offer
        .exchange_offer("aW5pdGlhdG9yLXBheWxvYWQ=", Some(300))
        .expect("exchange offer via OHTTP");
    assert!(!code.is_empty(), "exchange code must not be empty");

    // Responder: claim offer — returns the initiator's payload
    let t_claim = create_ohttp_transport(&ohttp_url, &key_bytes);
    let initiator_payload = t_claim
        .exchange_claim(&code, "cmVzcG9uZGVyLXBheWxvYWQ=")
        .expect("exchange claim via OHTTP");
    assert_eq!(
        initiator_payload, "aW5pdGlhdG9yLXBheWxvYWQ=",
        "claimed payload must match offer"
    );

    // Initiator: complete exchange — returns the responder's payload
    let t_complete = create_ohttp_transport(&ohttp_url, &key_bytes);
    let responder_payload = t_complete
        .exchange_complete(&code)
        .expect("exchange complete via OHTTP");
    assert_eq!(
        responder_payload.as_deref(),
        Some("cmVzcG9uZGVyLXBheWxvYWQ="),
        "completed payload must match claim"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P1: Fail-closed without OHTTP ──────────────────────────────────

// @scenario: sync:OHTTP fail-closed
#[tokio::test]
async fn test_fail_closed_without_ohttp() {
    // Transport with allow_direct=false and no OHTTP client must refuse requests.
    let config = HttpTransportConfig {
        relay_url: "http://127.0.0.1:1".to_string(),
        timeout_ms: 1_000,
        proxy: ProxyConfig::None,
        allow_direct: false,
    };
    let transport = HttpTransport::new(config);

    let result = transport.send_update(&"a".repeat(64), "dGVzdA==");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("OHTTP not configured"),
        "should fail-closed without OHTTP, got: {err}"
    );
}

// ── P2: Error cases ───���────────────────────────────────────────────

// @scenario: sync:OHTTP stale key
#[tokio::test]
async fn test_ohttp_with_garbage_key_returns_error() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) = spawn_ohttp_stack().await;

    // Use a valid-format but wrong key (generate a fresh one not matching the relay)
    let wrong_key = {
        use ohttp::{KeyConfig, SymmetricSuite, hpke};
        let config = KeyConfig::new(
            0,
            hpke::Kem::X25519Sha256,
            vec![SymmetricSuite::new(
                hpke::Kdf::HkdfSha256,
                hpke::Aead::Aes128Gcm,
            )],
        )
        .unwrap();
        config.encode().unwrap()
    };

    let transport = create_ohttp_transport(&ohttp_url, &wrong_key);
    let result = transport.send_update(&"a".repeat(64), "dGVzdA==");

    // Should fail — the relay can't decapsulate a blob encrypted with the wrong key
    assert!(result.is_err(), "wrong OHTTP key should produce an error");

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}
