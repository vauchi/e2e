// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for OHTTP E2E tests.
//!
//! Extracted from `ohttp_integration.rs` and `ohttp_advanced.rs` to
//! eliminate duplication of stack-spawn and transport-creation logic.

use vauchi_core::network::{HttpTransport, HttpTransportConfig, OhttpClient, ProxyConfig};
use vauchi_e2e_tests::ohttp_relay_manager::{OhttpRelayConfig, OhttpRelayManager};
use vauchi_e2e_tests::relay_manager::{RelayConfig, RelayManager};

/// Spawn vauchi-relay with OHTTP enabled + ohttp-relay forwarding proxy.
/// Returns (relay_manager, ohttp_relay_manager, relay_http_url, ohttp_relay_url).
pub async fn spawn_ohttp_stack() -> (RelayManager, OhttpRelayManager, String, String) {
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

/// Spawn a relay with fast key rotation (3s interval) + ohttp-relay
/// with caching disabled. Use `ROTATION_SECS` and `ROTATION_WAIT_SECS`
/// for consistent timing across tests.
pub async fn spawn_ohttp_stack_fast_rotation() -> (RelayManager, OhttpRelayManager, String, String)
{
    let relay_config = RelayConfig {
        http_api_enabled: true,
        ohttp_enabled: true,
        ohttp_key_rotation_hours: 1,
        ohttp_key_rotation_secs: Some(ROTATION_SECS),
        ..Default::default()
    };

    let mut relay_mgr = RelayManager::with_config(relay_config)
        .await
        .expect("relay manager");
    relay_mgr.spawn(1).await.expect("spawn relay");

    let relay_http_url = relay_mgr.relay(0).expect("relay instance").http_url();

    let ohttp_config = OhttpRelayConfig {
        key_cache_ttl_secs: 0,
        ..OhttpRelayConfig::default()
    };
    let mut ohttp_mgr = OhttpRelayManager::new(ohttp_config).expect("ohttp relay manager");
    ohttp_mgr
        .spawn(&relay_http_url)
        .await
        .expect("spawn ohttp-relay");

    let ohttp_url = ohttp_mgr.url().expect("ohttp relay url");

    (relay_mgr, ohttp_mgr, relay_http_url, ohttp_url)
}

/// Spawn a relay with fast rotation (3s) and an ohttp-relay with a short
/// cache (3s). Exercises the production-like path where the proxy caches keys.
pub async fn spawn_ohttp_stack_cached_rotation() -> (RelayManager, OhttpRelayManager, String, String)
{
    let relay_config = RelayConfig {
        http_api_enabled: true,
        ohttp_enabled: true,
        ohttp_key_rotation_hours: 1,
        ohttp_key_rotation_secs: Some(ROTATION_SECS),
        ..Default::default()
    };

    let mut relay_mgr = RelayManager::with_config(relay_config)
        .await
        .expect("relay manager");
    relay_mgr.spawn(1).await.expect("spawn relay");

    let relay_http_url = relay_mgr.relay(0).expect("relay instance").http_url();

    let ohttp_config = OhttpRelayConfig {
        key_cache_ttl_secs: ROTATION_SECS,
        ..OhttpRelayConfig::default()
    };
    let mut ohttp_mgr = OhttpRelayManager::new(ohttp_config).expect("ohttp relay manager");
    ohttp_mgr
        .spawn(&relay_http_url)
        .await
        .expect("spawn ohttp-relay");

    let ohttp_url = ohttp_mgr.url().expect("ohttp relay url");

    (relay_mgr, ohttp_mgr, relay_http_url, ohttp_url)
}

/// Create an HttpTransport configured to use the ohttp-relay, with OHTTP
/// encryption active.
pub fn create_ohttp_transport(ohttp_relay_url: &str, ohttp_key: &[u8]) -> HttpTransport {
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

/// Key rotation interval for fast-rotation tests (seconds).
pub const ROTATION_SECS: u64 = 3;

/// How long to sleep to guarantee at least one rotation has occurred.
/// Must be strictly greater than `ROTATION_SECS` with enough margin
/// for CI machines under load.
pub const ROTATION_WAIT_SECS: u64 = 5;
