// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test orchestrator for E2E testing.
//!
//! Provides lower-level coordination of relays, users, and devices
//! when the Scenario DSL doesn't meet specific needs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, info};

use crate::error::{E2eError, E2eResult};
use crate::ohttp_relay_manager::{OhttpRelayConfig, OhttpRelayManager};
use crate::relay_manager::{RelayConfig, RelayManager};
use crate::user::{User, UserBuilder};

/// Env var read by the cli (`cli/src/commands/common.rs`) to override
/// `OhttpConfig::bundled_gateway_key`. The orchestrator fetches the
/// freshly-spawned local relay's `/v2/ohttp-key` and sets this env on
/// every spawned `vauchi` subprocess so the release cli can encap to a
/// key the local relay can decrypt. See problem record
/// `_private/docs/problems/2026-05-04-f13-cli-bundled-key-injection-for-e2e/`.
const CLI_BUNDLED_OHTTP_KEY_HEX_ENV: &str = "VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX";

/// Configuration for the orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Relay configuration.
    pub relay_config: RelayConfig,
    /// Number of relays to spawn.
    pub relay_count: usize,
    /// Delay between operations (for observability).
    pub operation_delay: Duration,
    /// Spawn an `OhttpRelayManager` (the outer privacy hop) alongside
    /// the relay, and route CLI traffic through it. When `false` (default
    /// today), the CLI talks directly to the relay's OHTTP gateway —
    /// the OHTTP envelope is still encrypted, but the gateway operator
    /// sees the client IP.
    ///
    /// Production runs every request through an outer ohttp-relay per
    /// ADR-037 (gateway and forwarding-relay must be distinct
    /// entities). Setting this to `true` makes the test harness exercise
    /// the same path, catching outer-hop regressions (proxy caching,
    /// rate limiting, header forwarding) before release.
    ///
    /// Source record: `_private/docs/problems/2026-04-27-e2e-ohttp-default/`.
    pub with_ohttp_relay: bool,
    /// Configuration for the spawned ohttp-relay (only used when
    /// `with_ohttp_relay` is `true`).
    pub ohttp_relay_config: OhttpRelayConfig,
    /// Inject the spawned local relay's OHTTP gateway key into every
    /// cli subprocess via `VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX`.
    /// Required for release cli tests — the release binary compiles out
    /// the `VAUCHI_ALLOW_DIRECT` escape hatch and would otherwise fall
    /// back to the compiled-in production bundled key, whose pubkey the
    /// ephemeral local relay cannot decrypt.
    ///
    /// On by default so ordinary CLI-driven scenarios exercise the real
    /// OHTTP path against the local relay. Explicitly set to `false` to
    /// test key bootstrap (the client must fetch the live gateway key
    /// through the ohttp-relay). See problem record
    /// `2026-05-04-f13-cli-bundled-key-injection-for-e2e`.
    pub inject_local_ohttp_key_into_cli: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            relay_config: RelayConfig::default(),
            relay_count: 1,
            operation_delay: Duration::from_millis(100),
            with_ohttp_relay: true,
            ohttp_relay_config: OhttpRelayConfig::default(),
            inject_local_ohttp_key_into_cli: true,
        }
    }
}

impl OrchestratorConfig {
    /// Configure a multi-relay transport-isolation scenario.
    ///
    /// The outer OHTTP manager currently maps to one application relay, so
    /// tests whose subject is application-relay failover must opt into this
    /// narrower direct topology explicitly. Ordinary scenarios use OHTTP.
    pub fn multi_relay_transport_isolation(relay_count: usize) -> Self {
        Self {
            relay_count,
            with_ohttp_relay: false,
            ..Self::default()
        }
    }
}

/// E2E test orchestrator.
///
/// Provides direct control over relays, users, and devices for complex
/// test scenarios that don't fit the Scenario DSL pattern.
pub struct Orchestrator {
    config: OrchestratorConfig,
    relay_manager: Option<RelayManager>,
    ohttp_relay_manager: Option<OhttpRelayManager>,
    users: HashMap<String, Arc<RwLock<User>>>,
    started: bool,
    /// Hex-encoded local relay gateway key, populated on `start()` when
    /// `inject_local_ohttp_key_into_cli` is `true`. Forwarded to every
    /// spawned cli subprocess via `VAUCHI_OVERRIDE_BUNDLED_OHTTP_KEY_HEX`.
    cli_bundled_ohttp_key_hex: Option<String>,
}

impl Orchestrator {
    /// Create a new orchestrator with default configuration.
    pub fn new() -> Self {
        Self::with_config(OrchestratorConfig::default())
    }

    /// Create a new orchestrator with custom configuration.
    pub fn with_config(config: OrchestratorConfig) -> Self {
        Self {
            config,
            relay_manager: None,
            ohttp_relay_manager: None,
            users: HashMap::new(),
            started: false,
            cli_bundled_ohttp_key_hex: None,
        }
    }

    /// Start the orchestrator (spawn relays, optionally an
    /// `OhttpRelayManager` per `OrchestratorConfig::with_ohttp_relay`).
    ///
    /// When the outer ohttp-relay is enabled, multi-relay configurations
    /// (`relay_count > 1`) currently fan out to a single ohttp-relay
    /// pointing at relay 0; supporting per-relay outer hops is a Phase
    /// 2 follow-up. Tests that need multiple relays today should
    /// disable `with_ohttp_relay`.
    pub async fn start(&mut self) -> E2eResult<()> {
        if self.started {
            return Err(E2eError::scenario("Orchestrator already started"));
        }

        info!(
            "Starting orchestrator with {} relay(s){}",
            self.config.relay_count,
            if self.config.with_ohttp_relay {
                " + ohttp-relay"
            } else {
                ""
            }
        );

        let mut relay_manager = RelayManager::with_config(self.config.relay_config.clone()).await?;
        relay_manager.spawn(self.config.relay_count).await?;

        if self.config.with_ohttp_relay {
            if self.config.relay_count > 1 {
                return Err(E2eError::scenario(
                    "with_ohttp_relay does not yet support relay_count > 1 \
                     (see Phase 2 of 2026-04-27-e2e-ohttp-default)",
                ));
            }
            let upstream = relay_manager.relay_http_url(0).ok_or_else(|| {
                E2eError::scenario("relay HTTP URL unavailable for ohttp-relay forwarding")
            })?;
            let upstream = upstream.to_string();
            let mut ohttp_mgr = OhttpRelayManager::new(self.config.ohttp_relay_config.clone())?;
            ohttp_mgr.spawn(&upstream).await?;
            self.ohttp_relay_manager = Some(ohttp_mgr);
        }

        self.relay_manager = Some(relay_manager);

        // Fetch the freshly-spawned local relay's OHTTP gateway key
        // and stash it as the bundled-key override for every cli we
        // launch. Without this, the release cli (`E2E_BIN_DIR/vauchi`,
        // which compiles out the `VAUCHI_ALLOW_DIRECT` hatch) would
        // fall back to the production bundled key whose pubkey the
        // local relay's ephemeral private key cannot decrypt — see
        // F13 step 5 caveat in
        // `2026-05-04-ohttp-gateway-decap-unsupported-via-outer-hop`.
        if self.config.inject_local_ohttp_key_into_cli {
            let relay_http_url = self.primary_relay_http_url()?;
            let key_url = format!("{}/v2/ohttp-key", relay_http_url.trim_end_matches('/'));
            debug!("Fetching local relay OHTTP gateway key from {key_url}");
            let bytes = reqwest::get(&key_url)
                .await
                .map_err(|e| E2eError::scenario(format!("Failed to fetch {key_url}: {e}")))?
                .error_for_status()
                .map_err(|e| E2eError::scenario(format!("{key_url} returned non-2xx: {e}")))?
                .bytes()
                .await
                .map_err(|e| E2eError::scenario(format!("Failed to read {key_url} body: {e}")))?;
            self.cli_bundled_ohttp_key_hex = Some(hex::encode(&bytes));
            info!(
                "Injecting local relay OHTTP gateway key into cli subprocesses ({} bytes)",
                bytes.len()
            );
        }

        self.started = true;

        Ok(())
    }

    /// Stop the orchestrator (cleanup ohttp-relay first, then relays —
    /// tear down outside-in so the proxy doesn't outlive its upstream).
    pub async fn stop(&mut self) -> E2eResult<()> {
        if let Some(mut ohttp_mgr) = self.ohttp_relay_manager.take() {
            ohttp_mgr.stop().await;
        }
        if let Some(mut relay_manager) = self.relay_manager.take() {
            info!("Stopping orchestrator");
            relay_manager.stop_all().await;
        }
        self.started = false;
        Ok(())
    }

    /// Check if the orchestrator is running.
    pub fn is_running(&self) -> bool {
        self.started
    }

    /// Get the primary relay URL (WebSocket).
    pub fn primary_relay_url(&self) -> E2eResult<String> {
        self.relay_manager
            .as_ref()
            .and_then(|rm| rm.relay_url(0))
            .map(|s| s.to_string())
            .ok_or_else(|| E2eError::scenario("No relay available"))
    }

    /// Get the primary relay HTTP API URL.
    ///
    /// The v2 endpoints (OHTTP, exchange, sync) are served on the
    /// HTTP/metrics port, not the WebSocket port.
    pub fn primary_relay_http_url(&self) -> E2eResult<String> {
        self.relay_manager
            .as_ref()
            .and_then(|rm| rm.relay_http_url(0))
            .map(|s| s.to_string())
            .ok_or_else(|| E2eError::scenario("No relay HTTP API available"))
    }

    /// Get the URL the CLI should use as its `--relay`.
    ///
    /// When `OrchestratorConfig::with_ohttp_relay` is `true`, returns
    /// the spawned ohttp-relay's URL — the CLI then talks
    /// (client → ohttp-relay → relay-gateway), matching the
    /// production-like ADR-037 path. Otherwise returns the direct
    /// relay HTTP URL — the CLI's OHTTP envelope still encrypts
    /// payloads, but the gateway operator sees the client IP.
    ///
    /// New code should prefer this over `primary_relay_http_url()`
    /// — it transparently routes through the outer hop when one is
    /// configured.
    pub fn primary_cli_relay_url(&self) -> E2eResult<String> {
        if let Some(ohttp_mgr) = self.ohttp_relay_manager.as_ref()
            && let Some(url) = ohttp_mgr.url()
        {
            return Ok(url);
        }
        self.primary_relay_http_url()
    }

    /// Get the spawned ohttp-relay URL, if `with_ohttp_relay` is active.
    pub fn ohttp_relay_url(&self) -> Option<String> {
        self.ohttp_relay_manager.as_ref().and_then(|m| m.url())
    }

    /// Get all relay URLs.
    pub fn all_relay_urls(&self) -> Vec<String> {
        self.relay_manager
            .as_ref()
            .map(|rm| rm.all_urls().iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    /// Get a relay URL by index.
    pub fn relay_url(&self, index: usize) -> E2eResult<String> {
        self.relay_manager
            .as_ref()
            .and_then(|rm| rm.relay_url(index))
            .map(|s| s.to_string())
            .ok_or_else(|| E2eError::scenario(format!("Relay {} not available", index)))
    }

    /// Add a user with the specified number of devices.
    pub fn add_user(
        &mut self,
        name: impl Into<String>,
        device_count: usize,
    ) -> E2eResult<Arc<RwLock<User>>> {
        let name = name.into();
        // CLI uses HTTP transport — when `with_ohttp_relay` is on, this
        // returns the outer hop's URL (CLI → ohttp-relay → gateway);
        // otherwise the direct relay HTTP URL (CLI → gateway).
        let relay_url = self.primary_cli_relay_url()?;

        info!("Adding user '{}' with {} device(s)", name, device_count);

        let mut extra_env = HashMap::new();
        if let Some(hex) = self.cli_bundled_ohttp_key_hex.as_ref() {
            extra_env.insert(CLI_BUNDLED_OHTTP_KEY_HEX_ENV.to_string(), hex.clone());
        }

        let user = UserBuilder::new(&name, relay_url)
            .with_devices(device_count)
            .with_extra_env(extra_env)
            .build()?;

        let user = Arc::new(RwLock::new(user));
        self.users.insert(name, user.clone());

        Ok(user)
    }

    /// Add a user configured **production-shaped**: `--relay` points at the
    /// data relay's HTTP URL, and OHTTP traffic is routed through the
    /// *distinct* ohttp-relay via `VAUCHI_OHTTP_RELAY_URL`.
    ///
    /// This mirrors production (`relay.vauchi.app` + a separate
    /// `ohttp.vauchi.app`) so the client's OHTTP key bootstrap + routing are
    /// exercised end-to-end — the exact split that `add_user` (which points
    /// `--relay` straight at the ohttp-relay) does not cover. Deliberately
    /// injects **no** bundled-key override: the client must fetch the live
    /// gateway key through the ohttp-relay. Requires `with_ohttp_relay = true`.
    /// Regression guard for `2026-05-25-relay-ohttp-forward-hop-502`.
    pub fn add_user_split_ohttp(
        &mut self,
        name: impl Into<String>,
        device_count: usize,
    ) -> E2eResult<Arc<RwLock<User>>> {
        let name = name.into();
        let relay_url = self.primary_relay_http_url()?;
        let ohttp_url = self.ohttp_relay_url().ok_or_else(|| {
            E2eError::relay("add_user_split_ohttp requires OrchestratorConfig::with_ohttp_relay")
        })?;

        info!(
            "Adding split-OHTTP user '{}' with {} device(s) (relay + distinct ohttp-relay)",
            name, device_count
        );

        let mut extra_env = HashMap::new();
        extra_env.insert("VAUCHI_OHTTP_RELAY_URL".to_string(), ohttp_url);

        let user = UserBuilder::new(&name, relay_url)
            .with_devices(device_count)
            .with_extra_env(extra_env)
            .build()?;

        let user = Arc::new(RwLock::new(user));
        self.users.insert(name, user.clone());

        Ok(user)
    }

    /// Get a user by name.
    pub fn user(&self, name: &str) -> Option<Arc<RwLock<User>>> {
        self.users.get(name).cloned()
    }

    /// Get all users.
    pub fn users(&self) -> impl Iterator<Item = Arc<RwLock<User>>> + '_ {
        self.users.values().cloned()
    }

    /// Get the number of users.
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// Create identities for all users.
    pub async fn create_all_identities(&self) -> E2eResult<()> {
        for user in self.users() {
            let user = user.read().await;
            user.create_identity().await?;
            sleep(self.config.operation_delay).await;
        }
        Ok(())
    }

    /// Link all devices for all users.
    pub async fn link_all_devices(&self) -> E2eResult<()> {
        for user in self.users() {
            let user = user.read().await;
            user.link_devices().await?;
            sleep(self.config.operation_delay).await;
        }
        Ok(())
    }

    /// Sync all users' devices.
    pub async fn sync_all(&self) -> E2eResult<()> {
        for user in self.users() {
            let user = user.read().await;
            user.sync_all().await?;
        }
        Ok(())
    }

    /// Stop a specific relay (for failover testing).
    pub async fn stop_relay(&mut self, index: usize) -> E2eResult<()> {
        if let Some(rm) = &mut self.relay_manager {
            rm.stop_relay(index).await?;
        }
        Ok(())
    }

    /// Restart a specific relay (for failover testing).
    pub async fn restart_relay(&mut self, index: usize) -> E2eResult<()> {
        if let Some(rm) = &mut self.relay_manager {
            rm.restart_relay(index).await?;
        }
        Ok(())
    }

    /// Wait for a specified duration (for timing-sensitive tests).
    pub async fn wait(&self, duration: Duration) {
        debug!("Waiting for {:?}", duration);
        sleep(duration).await;
    }

    /// Perform a full exchange between two users.
    pub async fn exchange(&self, user_a_name: &str, user_b_name: &str) -> E2eResult<()> {
        let user_a = self
            .user(user_a_name)
            .ok_or_else(|| E2eError::user(format!("User '{}' not found", user_a_name)))?;
        let user_b = self
            .user(user_b_name)
            .ok_or_else(|| E2eError::user(format!("User '{}' not found", user_b_name)))?;

        info!("Mutual exchange: {} <-> {}", user_a_name, user_b_name);

        // User A generates QR, User B completes
        let qr_a = {
            let user = user_a.read().await;
            user.generate_qr().await?
        };
        {
            let user = user_b.read().await;
            user.complete_exchange(&qr_a).await?;
        }

        // User B generates QR, User A completes
        let qr_b = {
            let user = user_b.read().await;
            user.generate_qr().await?
        };
        {
            let user = user_a.read().await;
            user.complete_exchange(&qr_b).await?;
        }

        {
            let user = user_a.read().await;
            user.sync_all().await?;
        }
        {
            let user = user_b.read().await;
            user.sync_all().await?;
        }

        Ok(())
    }

    /// Perform exchanges between all users (creates a fully connected graph).
    pub async fn exchange_all(&self) -> E2eResult<()> {
        let names: Vec<String> = self.users.keys().cloned().collect();

        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                self.exchange(&names[i], &names[j]).await?;
                sleep(self.config.operation_delay).await;
            }
        }

        Ok(())
    }

    /// Verify that a user has a specific number of contacts on the primary device.
    ///
    /// Only checks the primary device because inter-device contact sync is not
    /// yet implemented (#38). Secondary devices don't receive contacts from
    /// exchanges performed on the primary.
    pub async fn verify_contact_count(&self, user_name: &str, expected: usize) -> E2eResult<()> {
        let user = self
            .user(user_name)
            .ok_or_else(|| E2eError::user(format!("User '{}' not found", user_name)))?;

        let user = user.read().await;
        let contacts = user.list_contacts().await?;

        if contacts.len() != expected {
            return Err(E2eError::assertion(format!(
                "User '{}' primary device has {} contacts, expected {}",
                user_name,
                contacts.len(),
                expected
            )));
        }

        debug!(
            "User '{}' verified: {} contacts on primary device",
            user_name, expected
        );

        Ok(())
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        // Note: Async cleanup happens in the relay_manager's Drop
    }
}

// INLINE_TEST_REQUIRED: unit tests for Orchestrator config/construction
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_config_default() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.relay_count, 1);
        assert!(
            config.with_ohttp_relay,
            "ordinary E2E scenarios must exercise the outer OHTTP hop by default"
        );
    }

    #[test]
    fn multi_relay_transport_isolation_is_an_explicit_direct_mode() {
        let config = OrchestratorConfig::multi_relay_transport_isolation(2);
        assert_eq!(config.relay_count, 2);
        assert!(!config.with_ohttp_relay);
    }

    #[test]
    fn test_orchestrator_new() {
        let orch = Orchestrator::new();
        assert!(!orch.is_running());
        assert_eq!(orch.user_count(), 0);
    }
}
