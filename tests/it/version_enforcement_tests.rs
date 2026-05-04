// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! E2E tests for C8 version enforcement round-trip.
//!
//! Validates the full pipeline: client sends `X-App-Compat-Version` header →
//! relay middleware checks version policy → response headers/status come back.
//!
//! Each test spawns a real relay binary with specific version policy env vars
//! and exercises the HTTP API directly with `reqwest`.

use vauchi_e2e_tests::relay_manager::{RelayConfig, RelayManager};

/// Spawn a relay with the given version policy and return (manager, http_url).
async fn spawn_relay_with_version_policy(
    min: u16,
    warn: u16,
    grace_days: u16,
) -> (RelayManager, String) {
    spawn_relay_with_version_policy_aged(min, warn, grace_days, None).await
}

/// Variant that lets the test set `min_version_changed_at` to an
/// arbitrary unix timestamp (via `RELAY_VERSION_CHANGED_AT_SECS`) so
/// it can simulate "grace already expired" or any other temporal
/// position. `aged_at_secs = Some(0)` puts the change at unix epoch
/// — guarantees grace expired regardless of `grace_days`. Production
/// must NOT set this env var; the relay logs WARN if it does.
async fn spawn_relay_with_version_policy_aged(
    min: u16,
    warn: u16,
    grace_days: u16,
    aged_at_secs: Option<u64>,
) -> (RelayManager, String) {
    let config = RelayConfig {
        http_api_enabled: true,
        ohttp_enabled: false,
        version_min: Some(min),
        version_warn: Some(warn),
        version_grace_days: Some(grace_days),
        version_changed_at_secs: aged_at_secs,
        ..Default::default()
    };

    let mut mgr = RelayManager::with_config(config)
        .await
        .expect("relay manager");
    mgr.spawn(1).await.expect("spawn relay");

    let http_url = mgr.relay(0).expect("relay instance").http_url();
    (mgr, http_url)
}

// ── Scenario 1: Current client connects normally ──────────────────

// @internal
/// Client sends version matching `APP_COMPAT_VERSION` (1). Relay has min=1, warn=1.
/// Should get 200 OK with version headers.
// @internal
#[tokio::test]
async fn current_client_connects_normally() {
    let (_mgr, http_url) = spawn_relay_with_version_policy(1, 1, 14).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{http_url}/v2/health"))
        .header("X-App-Compat-Version", "1")
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "current client should get 200 OK"
    );

    // Verify version headers are present in the response.
    let min_hdr = resp
        .headers()
        .get("X-Min-Version")
        .expect("response should contain X-Min-Version header")
        .to_str()
        .expect("header should be valid UTF-8");
    assert_eq!(min_hdr, "1", "X-Min-Version should be 1");

    let warn_hdr = resp
        .headers()
        .get("X-Warn-Version")
        .expect("response should contain X-Warn-Version header")
        .to_str()
        .expect("header should be valid UTF-8");
    assert_eq!(warn_hdr, "1", "X-Warn-Version should be 1");

    // No upgrade deadline when client version meets minimum.
    assert!(
        resp.headers().get("X-Upgrade-Deadline").is_none(),
        "should not have X-Upgrade-Deadline when version is at or above min"
    );
}

// ── Scenario 2: Old client rejected (grace expired) ──────────────

// @internal
/// Relay has min=99 (above any real client). The
/// `version_changed_at_secs = Some(0)` override pegs
/// `min_version_changed_at` at the unix epoch via
/// `RELAY_VERSION_CHANGED_AT_SECS` — guarantees grace has long expired
/// regardless of `grace_days`. Old client (version=1) is rejected
/// with HTTP 426. Pins the rejection path of the version-policy
/// state machine.
///
/// Harness shipped per `2026-04-27-version-enforcement-tests-fail`
/// Option A (env-var override).
// @internal
#[tokio::test]
async fn old_client_rejected_without_grace() {
    let (_mgr, http_url) = spawn_relay_with_version_policy_aged(99, 99, 1, Some(0)).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{http_url}/v2/health"))
        .header("X-App-Compat-Version", "1")
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        426,
        "old client should get 426 Upgrade Required"
    );

    // Verify the JSON body contains the expected error payload.
    let body: serde_json::Value = resp.json().await.expect("response should be valid JSON");
    assert_eq!(
        body["error"], "upgrade_required",
        "error field should be 'upgrade_required'"
    );
    assert_eq!(
        body["min_version"], 99,
        "min_version field should match relay config"
    );
}

// ── Scenario 3: Client receives warning headers ──────────────────

// @internal
/// Relay has min=1, warn=99. Client sends version 1 (at min, below warn).
/// Should get 200 with `X-Min-Version: 1` and `X-Warn-Version: 99`.
// @internal
#[tokio::test]
async fn client_receives_warning_headers() {
    let (_mgr, http_url) = spawn_relay_with_version_policy(1, 99, 14).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{http_url}/v2/health"))
        .header("X-App-Compat-Version", "1")
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "client at min version should get 200 OK"
    );

    let min_hdr = resp
        .headers()
        .get("X-Min-Version")
        .expect("response should contain X-Min-Version header")
        .to_str()
        .expect("header should be valid UTF-8");
    assert_eq!(min_hdr, "1", "X-Min-Version should be 1");

    let warn_hdr = resp
        .headers()
        .get("X-Warn-Version")
        .expect("response should contain X-Warn-Version header")
        .to_str()
        .expect("header should be valid UTF-8");
    assert_eq!(warn_hdr, "99", "X-Warn-Version should be 99");
}

// ── Scenario 4: Missing header treated as version 0 ──────────────

// @internal
/// Relay has min=1, grace_days=14, with `min_version_changed_at`
/// pegged at unix epoch via the test override — so the grace window
/// expired ~56 years ago. Client sends no `X-App-Compat-Version`
/// header → treated as version 0 → below min → rejected with 426
/// (no grace remaining).
///
/// Harness shipped per `2026-04-27-version-enforcement-tests-fail`
/// Option A.
// @internal
#[tokio::test]
async fn missing_version_header_treated_as_zero() {
    let (_mgr, http_url) = spawn_relay_with_version_policy_aged(1, 1, 14, Some(0)).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{http_url}/v2/health"))
        // No X-App-Compat-Version header.
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        426,
        "missing version header should be treated as version 0 and rejected"
    );

    let body: serde_json::Value = resp.json().await.expect("response should be valid JSON");
    assert_eq!(body["error"], "upgrade_required");
}

// ── Scenario 5: No enforcement when min=0 ────────────────────────

// @internal
/// Relay has min=0, warn=0 (default). Even clients without the version header
/// should be allowed through.
// @internal
#[tokio::test]
async fn no_enforcement_when_min_is_zero() {
    let (_mgr, http_url) = spawn_relay_with_version_policy(0, 0, 14).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{http_url}/v2/health"))
        // No version header — should still pass when min=0.
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "min=0 should allow all clients"
    );

    let min_hdr = resp
        .headers()
        .get("X-Min-Version")
        .expect("response should contain X-Min-Version header")
        .to_str()
        .expect("header should be valid UTF-8");
    assert_eq!(min_hdr, "0");
}

// ── Scenario 6: Within-grace client gets deadline header ─────────

// @internal
/// Relay has min=99 (above any real client) and `grace_days=14`.
/// Fresh relay persists `min_version_changed_at = now`, so the grace
/// period is active for 14 days. An old client (version=1) should be
/// allowed through with HTTP 200 plus an `X-Upgrade-Deadline` header
/// announcing when the grace expires.
///
/// Pins the `AllowedWithDeadline` enforcement branch so a future
/// regression that drops grace handling (returning 426 immediately)
/// would surface here instead of as a silent rollout outage.
// @internal
#[tokio::test]
async fn within_grace_client_gets_deadline_header() {
    let (_mgr, http_url) = spawn_relay_with_version_policy(99, 99, 14).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{http_url}/v2/health"))
        .header("X-App-Compat-Version", "1")
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "within-grace client should get 200 OK with deadline header"
    );

    assert!(
        resp.headers().get("X-Upgrade-Deadline").is_some(),
        "within-grace response should carry X-Upgrade-Deadline header"
    );

    let min_hdr = resp
        .headers()
        .get("X-Min-Version")
        .expect("response should contain X-Min-Version header")
        .to_str()
        .expect("header should be valid UTF-8");
    assert_eq!(min_hdr, "99", "X-Min-Version should match relay config");
}
