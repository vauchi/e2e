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
        pinned_certs: vec![],
    };
    let mut transport = HttpTransport::new(config);

    let client = OhttpClient::new(ohttp_key.to_vec()).expect("create OhttpClient from key");
    transport.set_ohttp(client);

    transport
}

// ── P1: Key Bootstrap ──────────────────────────────────────────────

// @scenario: sync:OHTTP key fetch
#[tokio::test]
async fn integration_ohttp_key_bootstrap_via_relay() {
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
async fn integration_ohttp_send_and_fetch() {
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

    // 5. Fetch via OHTTP (same transport could be reused — OhttpClient is stateless)
    let transport2 = create_ohttp_transport(&ohttp_url, &key_bytes);
    let fetched = transport2.fetch(std::slice::from_ref(&recipient_id));

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
    let after_blobs = transport4
        .fetch(&[recipient_id])
        .expect("fetch after ack should succeed");
    assert!(
        after_blobs.is_empty(),
        "fetch after ack should return no blobs"
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P1: Exchange via OHTTP ─────────────────────────────────────────

// @scenario: exchange:OHTTP relay exchange
#[tokio::test]
async fn integration_ohttp_exchange_offer_claim_complete() {
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
async fn integration_ohttp_fail_closed_without_config() {
    // Transport with allow_direct=false and no OHTTP client must refuse requests.
    let config = HttpTransportConfig {
        relay_url: "http://127.0.0.1:1".to_string(),
        timeout_ms: 1_000,
        proxy: ProxyConfig::None,
        allow_direct: false,
        pinned_certs: vec![],
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

// ── P1.5: IP Obfuscation Verification ─────────────────────────────

// @scenario: sync:OHTTP IP obfuscation
#[tokio::test]
async fn integration_ohttp_relay_strips_client_identity() {
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Headers that, if forwarded, would leak the client's identity.
    const IDENTIFYING_HEADERS: &[&str] = &[
        "x-forwarded-for",
        "x-real-ip",
        "forwarded",
        "via",
        "x-client-ip",
        "cf-connecting-ip",
        "true-client-ip",
        "cookie",
        "authorization",
        "x-custom-client-id",
    ];

    /// Recorded metadata from a request received by the mock gateway.
    #[derive(Debug, Clone)]
    struct RecordedRequest {
        remote_addr: SocketAddr,
        headers: Vec<(String, String)>,
        method: String,
        uri: String,
    }

    // ── 1. Spawn a mock gateway that records all request metadata ──

    let recorded: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded_clone = recorded.clone();

    let mock_app = {
        use axum::{
            Router,
            body::Bytes,
            extract::ConnectInfo,
            http::StatusCode,
            routing::{get, post},
        };

        let rec = recorded_clone.clone();
        let ohttp_handler =
            move |ConnectInfo(addr): ConnectInfo<SocketAddr>,
                  request: axum::http::Request<axum::body::Body>| {
                let rec = rec.clone();
                async move {
                    let headers: Vec<(String, String)> = request
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
                        .collect();

                    rec.lock().await.push(RecordedRequest {
                        remote_addr: addr,
                        headers,
                        method: request.method().to_string(),
                        uri: request.uri().to_string(),
                    });

                    // Return a dummy OHTTP response (400 is fine — we only care about
                    // what headers arrived, not a valid OHTTP exchange).
                    StatusCode::BAD_REQUEST
                }
            };

        let rec2 = recorded_clone.clone();
        let key_handler =
            move |ConnectInfo(addr): ConnectInfo<SocketAddr>,
                  request: axum::http::Request<axum::body::Body>| {
                let rec2 = rec2.clone();
                async move {
                    let headers: Vec<(String, String)> = request
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
                        .collect();

                    rec2.lock().await.push(RecordedRequest {
                        remote_addr: addr,
                        headers,
                        method: "GET".to_string(),
                        uri: "/v2/ohttp-key".to_string(),
                    });

                    // Return a dummy key (enough to record the request metadata).
                    (
                        StatusCode::OK,
                        [("content-type", "application/ohttp-keys")],
                        Bytes::from_static(&[0xAA, 0xBB]),
                    )
                }
            };

        Router::new()
            .route("/v2/ohttp", post(ohttp_handler))
            .route("/v2/ohttp-key", get(key_handler))
            .route("/health", get(|| async { StatusCode::OK }))
    };

    let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let mock_addr = mock_listener.local_addr().expect("mock gateway addr");
    let mock_url = format!("http://{mock_addr}");

    tokio::spawn(async move {
        axum::serve(
            mock_listener,
            mock_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // ── 2. Spawn ohttp-relay pointing at the mock gateway ─────────

    let mut ohttp_mgr =
        OhttpRelayManager::new(OhttpRelayConfig::default()).expect("ohttp relay manager");
    ohttp_mgr.spawn(&mock_url).await.expect("spawn ohttp-relay");
    let ohttp_url = ohttp_mgr.url().expect("ohttp relay url");
    let ohttp_port = ohttp_mgr.instance().expect("instance").port;

    // ── 3. Client sends requests WITH identifying headers ─────────

    let client = reqwest::Client::new();

    // 3a. Key fetch with identifying headers
    let _key_resp = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .header("X-Forwarded-For", "192.168.1.42")
        .header("X-Real-Ip", "10.0.0.99")
        .header("Forwarded", "for=203.0.113.50")
        .header("Via", "1.1 client-proxy.example.com")
        .header("Cookie", "session=secret-token-12345")
        .header("Authorization", "Bearer client-jwt-token")
        .header("X-Custom-Client-Id", "unique-device-fingerprint")
        .send()
        .await
        .expect("key fetch through ohttp-relay");

    // 3b. OHTTP blob forward with identifying headers
    let _ohttp_resp = client
        .post(format!("{ohttp_url}/v2/ohttp"))
        .header("Content-Type", "message/ohttp-req")
        .header("X-Forwarded-For", "192.168.1.42")
        .header("X-Real-Ip", "10.0.0.99")
        .header("Forwarded", "for=203.0.113.50")
        .header("Via", "1.1 client-proxy.example.com")
        .header("Cookie", "session=secret-token-12345")
        .header("Authorization", "Bearer client-jwt-token")
        .header("X-Custom-Client-Id", "unique-device-fingerprint")
        .header("True-Client-Ip", "172.16.0.5")
        .header("Cf-Connecting-Ip", "198.51.100.78")
        .body(vec![0x01, 0x02, 0x03])
        .send()
        .await
        .expect("OHTTP forward through ohttp-relay");

    // ── 4. Verify: no client-identifying information reached the gateway ──

    let requests = recorded.lock().await;
    assert!(
        requests.len() >= 2,
        "mock gateway should have received at least 2 requests (key + ohttp), got {}",
        requests.len()
    );

    for (i, req) in requests.iter().enumerate() {
        // 4a. Remote address must be ohttp-relay, not the test client
        assert_eq!(
            req.remote_addr.ip(),
            std::net::IpAddr::from([127, 0, 0, 1]),
            "request {i} ({} {}): remote IP must be loopback",
            req.method,
            req.uri
        );
        // The connection must come from the ohttp-relay process, not the test client.
        // We can't check the exact ephemeral port, but we verify it's NOT the ohttp-relay's
        // listen port (that would mean the client connected directly to the gateway).
        assert_ne!(
            req.remote_addr.port(),
            ohttp_port,
            "request {i}: connection must NOT come from ohttp-relay's listen port \
             (that would indicate a misconfigured test, not a real forwarded connection)"
        );

        // 4b. No identifying headers must be present
        let header_names: Vec<&str> = req.headers.iter().map(|(k, _)| k.as_str()).collect();
        for &forbidden in IDENTIFYING_HEADERS {
            assert!(
                !header_names.contains(&forbidden),
                "request {i} ({} {}): identifying header '{}' was forwarded to the gateway — \
                 ohttp-relay must strip all client headers. Headers present: {:?}",
                req.method,
                req.uri,
                forbidden,
                header_names
            );
        }

        // 4c. Specifically verify none of the injected client IP values appear
        //     anywhere in the header values
        let client_ips = [
            "192.168.1.42",
            "10.0.0.99",
            "203.0.113.50",
            "172.16.0.5",
            "198.51.100.78",
        ];
        for (hdr_name, hdr_value) in &req.headers {
            for client_ip in &client_ips {
                assert!(
                    !hdr_value.contains(client_ip),
                    "request {i} ({} {}): client IP '{}' leaked in header '{}': '{}'",
                    req.method,
                    req.uri,
                    client_ip,
                    hdr_name,
                    hdr_value
                );
            }
        }
    }

    ohttp_mgr.stop().await;
}

// ── P1.4: Key Rotation Fallback ───────────────────────────────────

/// Spawn a relay with fast key rotation (2s interval) + ohttp-relay.
async fn spawn_ohttp_stack_fast_rotation() -> (RelayManager, OhttpRelayManager, String, String) {
    let relay_config = RelayConfig {
        http_api_enabled: true,
        ohttp_enabled: true,
        ohttp_key_rotation_hours: 1,
        ohttp_key_rotation_secs: Some(2),
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

// @scenario: sync:OHTTP key rotation fallback
#[tokio::test]
async fn integration_ohttp_key_rotation_grace_period() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_url, ohttp_url) =
        spawn_ohttp_stack_fast_rotation().await;

    let client = reqwest::Client::new();

    // 1. Fetch initial key K1
    let key_k1 = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch K1")
        .bytes()
        .await
        .expect("read K1");
    assert!(!key_k1.is_empty(), "K1 must not be empty");

    // 2. Send with K1 — should succeed (K1 is current)
    let transport_k1 = create_ohttp_transport(&ohttp_url, &key_k1);
    let result = transport_k1.send_update(&"a".repeat(64), "dGVzdA==");
    assert!(
        result.is_ok(),
        "send with current key K1 must succeed: {:?}",
        result.err()
    );

    // 3. Wait for key rotation (2s interval + margin)
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // 4. Verify key has actually rotated (new key != K1)
    let key_k2 = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch K2")
        .bytes()
        .await
        .expect("read K2");
    assert_ne!(
        key_k1.as_ref(),
        key_k2.as_ref(),
        "key must have rotated after waiting"
    );

    // 5. Send with stale K1 — should STILL succeed (S10 grace period)
    let transport_stale = create_ohttp_transport(&ohttp_url, &key_k1);
    let grace_result = transport_stale.send_update(&"b".repeat(64), "Z3JhY2U=");
    assert!(
        grace_result.is_ok(),
        "send with stale K1 must succeed via S10 grace period: {:?}",
        grace_result.err()
    );

    // 6. Wait for second rotation (K1 gets evicted, K2 becomes previous)
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // 7. Verify another rotation happened
    let key_k3 = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("fetch K3")
        .bytes()
        .await
        .expect("read K3");
    assert_ne!(
        key_k2.as_ref(),
        key_k3.as_ref(),
        "key must have rotated a second time"
    );

    // 8. Send with K1 — should FAIL (evicted after 2nd rotation)
    let transport_evicted = create_ohttp_transport(&ohttp_url, &key_k1);
    let evicted_result = transport_evicted.send_update(&"c".repeat(64), "ZXZpY3RlZA==");
    assert!(
        evicted_result.is_err(),
        "send with doubly-stale K1 must fail (key evicted after 2nd rotation)"
    );

    // 9. Refetch key and send — should succeed (client recovery)
    let key_fresh = client
        .get(format!("{ohttp_url}/v2/ohttp-key"))
        .send()
        .await
        .expect("refetch key")
        .bytes()
        .await
        .expect("read fresh key");
    let transport_fresh = create_ohttp_transport(&ohttp_url, &key_fresh);
    let fresh_result = transport_fresh.send_update(&"d".repeat(64), "ZnJlc2g=");
    assert!(
        fresh_result.is_ok(),
        "send with refetched key must succeed: {:?}",
        fresh_result.err()
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// ── P2: Error cases ───���────────────────────────────────────────────

// @scenario: sync:OHTTP stale key
#[tokio::test]
async fn integration_ohttp_with_garbage_key_returns_error() {
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

// ── Smoke: OHTTP key bootstrap + send ──────────────────────────────
// Minimal OHTTP smoke tests for MR gate.

// @scenario: sync:OHTTP key bootstrap + send
#[tokio::test]
async fn smoke_ohttp_key_bootstrap_and_send() {
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

    assert!(!key_bytes.is_empty(), "OHTTP key must not be empty");

    // 2. Verify key is valid
    let ohttp_client = OhttpClient::new(key_bytes.to_vec());
    assert!(
        ohttp_client.is_ok(),
        "OHTTP key must be valid: {:?}",
        ohttp_client.err()
    );

    // 3. Send via OHTTP
    let transport = create_ohttp_transport(&ohttp_url, &key_bytes);
    let result = transport.send_update(&"a".repeat(64), "dGVzdA==");
    assert!(result.is_ok(), "send via OHTTP failed: {:?}", result.err());

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}

// @scenario: sync:OHTTP fail-closed
#[tokio::test]
async fn smoke_ohttp_fail_closed() {
    // Transport with allow_direct=false and no OHTTP must refuse.
    let config = HttpTransportConfig {
        relay_url: "http://127.0.0.1:1".to_string(),
        timeout_ms: 1_000,
        proxy: ProxyConfig::None,
        allow_direct: false,
        pinned_certs: vec![],
    };
    let transport = HttpTransport::new(config);
    let result = transport.send_update(&"a".repeat(64), "dGVzdA==");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("OHTTP not configured"),
        "should fail-closed without OHTTP"
    );
}
