// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Smoke test for `OrchestratorConfig::with_ohttp_relay`.
//!
//! Phase 1 of `_private/docs/problems/2026-04-27-e2e-ohttp-default/` —
//! adds the infrastructure for the orchestrator to spawn an
//! `OhttpRelayManager` (the outer privacy hop per ADR-037) and route
//! the CLI through it. This test pins that the new config field
//! actually wires the spawned ohttp-relay into the path the CLI
//! uses, and that a basic exchange completes end-to-end through it.
//!
//! The outer privacy hop is the default. Transport-isolation tests must opt
//! out explicitly instead of allowing ordinary scenarios to bypass it.

use std::time::Duration;

use vauchi_e2e_tests::{
    orchestrator::{Orchestrator, OrchestratorConfig},
    relay_manager::RelayConfig,
};

/// Phase 1 smoke: orchestrator spawns ohttp-relay when configured,
/// routes the CLI through it, and a basic exchange completes.
///
/// 2026-05-04 history:
///
/// 1. **Routing 404** (initial finding). The CLI's adapter-level
///    `health_check()` hit `/v2/health` directly; the outer hop
///    didn't proxy that path. Fixed by `core!765`
///    (`refactor: drop adapter health-check probe (F12 Option A)`).
/// 2. **Decap "Unsupported" 502** (revealed after F12). With the
///    routing fixed, the gateway received encrypted envelopes but
///    rejected them with `OHTTP decapsulate failed: configuration
///    was not supported`. Root cause was the stale pre-ADR-046
///    `BUNDLED_OHTTP_KEY` advertising AES-128-GCM after relay!267
///    swapped the gateway to ChaCha20-Poly1305-only. Bundled key
///    regenerated in `core!766`; problem record
///    `2026-05-04-ohttp-gateway-decap-unsupported-via-outer-hop`.
/// 3. **Pubkey mismatch on local relay** (revealed by F13 step 5
///    analysis). After the cipher fix the cli encaps to the
///    *production* pubkey baked into the bundled key, but the
///    test harness spawns a fresh local relay with an **ephemeral**
///    keypair — decap still fails. The release E2E_BIN_DIR cli
///    compiles out the `VAUCHI_ALLOW_DIRECT` escape hatch, so the
///    fallback to the bundled production key is unconditional.
///    Fixed by orchestrator-injected key:
///    `OrchestratorConfig::inject_local_ohttp_key_into_cli` fetches
///    the local relay's `/v2/ohttp-key` once at start() and forwards
///    it to every spawned cli via `VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX`.
///    Problem record `2026-05-04-f13-cli-bundled-key-injection-for-e2e`.
/// 4. **Residual `HTTP 404` on `vauchi sync`** in CI (transient).
///    Step 5's first un-ignore attempt deterministically failed in
///    CI with `HTTP 404` while passing locally. Suspect 1 — stale CI
///    binary cache — confirmed by the next pipeline run on
///    investigation branch `investigation/f13-residual-404-ci-only`:
///    once CI's `smoke-binaries-*` cache rolled over to include
///    `cli@8880ebef` (the `VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX`
///    env hatch) and `ohttp-relay@33d34b8e` (with both `/v2/ohttp`
///    + `/v2/ohttp-key` routes registered), the smoke passed
///    end-to-end (12.0s) on the same code that previously failed.
///    Test un-ignored permanently in this MR. If 404s reappear after
///    a future cache invalidation, the suspect is again the cache
///    age, not the harness — the diagnostics added in
///    `investigation/f13-residual-404-ci-only` are the reference
///    repro. Closes step 5 of
///    `2026-05-04-ohttp-gateway-decap-unsupported-via-outer-hop`
///    + `2026-05-04-f13-cli-bundled-key-injection-for-e2e`.
// @scenario: ohttp_outer_hop :: cli completes a 2-user exchange via the orchestrator-spawned outer ohttp-relay
#[tokio::test]
async fn smoke_orchestrator_with_ohttp_relay_routes_through_outer_hop() {
    let config = OrchestratorConfig {
        relay_config: RelayConfig {
            ohttp_enabled: true,
            ..Default::default()
        },
        with_ohttp_relay: true,
        inject_local_ohttp_key_into_cli: true,
        ..Default::default()
    };
    let mut orch = Orchestrator::with_config(config);
    orch.start().await.expect("Failed to start orchestrator");

    // The CLI URL the orchestrator hands out should be the outer-hop
    // ohttp-relay URL, NOT the direct relay HTTP URL — this is the
    // load-bearing assertion: if the wiring regresses (e.g. the new
    // accessor falls back to the direct URL silently) we want this
    // test to fail loudly rather than the broader scenarios silently
    // bypass the outer hop.
    let cli_url = orch
        .primary_cli_relay_url()
        .expect("CLI relay URL should be available");
    let direct_url = orch
        .primary_relay_http_url()
        .expect("direct relay HTTP URL should be available");
    let outer_url = orch
        .ohttp_relay_url()
        .expect("ohttp-relay URL should be Some when with_ohttp_relay is on");

    assert_eq!(
        cli_url, outer_url,
        "CLI URL must be the ohttp-relay URL when with_ohttp_relay is on, \
         not {direct_url} — otherwise broader scenarios silently bypass the outer hop"
    );
    assert_ne!(
        cli_url, direct_url,
        "CLI URL must differ from direct relay URL when outer hop is active"
    );

    // End-to-end: a basic 2-user exchange must complete via the outer
    // hop. This is the regression seal — if the ohttp-relay forwarding
    // breaks (proxy caching, header forwarding, key bootstrap), the
    // exchange would fail here.
    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities via ohttp-relay");

    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange Alice ↔ Bob via ohttp-relay failed");

    let bob = orch.user("Bob").expect("Bob exists");
    let bob = bob.read().await;
    let contacts = bob
        .list_contacts()
        .await
        .expect("Failed to list Bob's contacts after ohttp-relay exchange");
    assert!(
        !contacts.is_empty(),
        "Bob should have Alice as a contact after exchange via ohttp-relay"
    );

    orch.stop().await.expect("Failed to stop orchestrator");

    // Belt-and-suspenders: give the OS a moment to fully release ports
    // before the next test in the suite reuses them. Same pattern as
    // existing OHTTP integration tests.
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Default configuration must spawn the outer hop and route the CLI through
/// it. This prevents broad scenarios from silently testing a weaker topology.
// @scenario: ohttp_outer_hop :: default orchestrator routes through the outer ohttp-relay
#[tokio::test]
async fn smoke_orchestrator_default_uses_ohttp_relay() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    let cli_url = orch
        .primary_cli_relay_url()
        .expect("CLI relay URL should be available");
    let direct_url = orch
        .primary_relay_http_url()
        .expect("direct relay HTTP URL should be available");

    assert_ne!(
        cli_url, direct_url,
        "default OrchestratorConfig must not route the CLI directly to the application relay"
    );
    assert!(
        orch.ohttp_relay_url().is_some(),
        "default OrchestratorConfig must spawn an ohttp-relay"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Production-mirror: the client is configured exactly like production —
/// `--relay` at the data relay, OHTTP routed through a *distinct*
/// ohttp-relay (`VAUCHI_OHTTP_RELAY_URL`), and **no** bundled-key
/// injection. A 2-user exchange + card-update sync must complete: the
/// client bootstraps the gateway key by fetching it through the ohttp-relay
/// and sends `POST /v2/ohttp` there.
///
/// Regression seal for `2026-05-25-relay-ohttp-forward-hop-502`: production
/// had no OHTTP endpoint configured, so the client POSTed OHTTP blobs to the
/// data relay (which doesn't serve `/v2/ohttp`) → kamal-proxy 502 → sync
/// never worked. With the Option B fix (`core!966` + `cli!280`) the client
/// routes OHTTP to the distinct ohttp-relay. If that regresses, `exchange`
/// / `sync_all` below fail loudly.
// @scenario: ohttp_outer_hop :: cli with split data/ohttp relay config completes exchange + card-update sync
#[tokio::test]
async fn integration_ohttp_split_relay_config_routes_via_ohttp_relay() {
    let config = OrchestratorConfig {
        relay_config: RelayConfig {
            ohttp_enabled: true,
            ..Default::default()
        },
        with_ohttp_relay: true,
        // Explicitly disable bundled-key injection so the client must fetch
        // the live gateway key through the ohttp-relay (the Option B path).
        inject_local_ohttp_key_into_cli: false,
        ..Default::default()
    };
    let mut orch = Orchestrator::with_config(config);
    orch.start().await.expect("Failed to start orchestrator");

    // The split must be real: the data relay URL and the ohttp-relay URL
    // must differ (mirrors relay.vauchi.app vs ohttp.vauchi.app). Users get
    // `--relay = data` + `VAUCHI_OHTTP_RELAY_URL = ohttp`.
    let data_url = orch
        .primary_relay_http_url()
        .expect("data relay HTTP URL should be available");
    let ohttp_url = orch
        .ohttp_relay_url()
        .expect("ohttp-relay URL should be Some when with_ohttp_relay is on");
    assert_ne!(
        data_url, ohttp_url,
        "production-mirror requires distinct data + ohttp-relay URLs"
    );

    orch.add_user_split_ohttp("Alice", 1)
        .expect("Failed to add Alice (split ohttp)");
    orch.add_user_split_ohttp("Bob", 1)
        .expect("Failed to add Bob (split ohttp)");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities (split ohttp)");
    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange via split data/ohttp config failed");

    // Decisive step: Alice updates her card and syncs. The OHTTP POST must
    // route to the ohttp-relay and the gateway key must have been fetched
    // through it — both of which 502'd in production before the fix.
    let alice = orch.user("Alice").expect("Alice exists");
    let bob = orch.user("Bob").expect("Bob exists");
    {
        let alice = alice.read().await;
        alice
            .add_field("email", "Email", "alice@example.com")
            .await
            .expect("Failed to add Alice's email field");
        alice
            .sync_all()
            .await
            .expect("Alice sync via split ohttp failed (the relay-502 path)");
    }
    {
        let bob = bob.read().await;
        bob.sync_all()
            .await
            .expect("Bob sync via split ohttp failed");
    }
    {
        let bob = bob.read().await;
        let alice_card = bob
            .get_contact_card("Alice")
            .await
            .expect("Failed to read Alice's synced card")
            .expect("Alice should be present after split-ohttp delivery");
        assert_eq!(alice_card.name, "Alice");
        assert_eq!(alice_card.fields.len(), 1);
        assert_eq!(alice_card.fields[0].field_type, "Email");
        assert_eq!(alice_card.fields[0].label, "Email");
        assert_eq!(alice_card.fields[0].value, "alice@example.com");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
    tokio::time::sleep(Duration::from_millis(200)).await;
}
