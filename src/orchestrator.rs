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
const CLI_OHTTP_RELAY_URL_ENV: &str = "VAUCHI_OHTTP_RELAY_URL";

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
    /// the relay, and route CLI traffic through it. When `false`, explicit
    /// transport-isolation scenarios identify the application via the main
    /// listener's HTTP origin and use the same relay's separate HTTP gateway
    /// origin for OHTTP.
    ///
    /// Production runs every request through an outer ohttp-relay per
    /// ADR-037 (gateway and forwarding-relay must be distinct
    /// entities). Setting this to `true` makes the test harness exercise
    /// the same path, catching outer-hop regressions (proxy caching and
    /// header forwarding) before release. Generic functional scenarios
    /// disable throttling; rate limiting has a dedicated integration test.
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
            ohttp_relay_config: OhttpRelayConfig {
                rate_limit_per_sec: 0,
                ..Default::default()
            },
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
            // Bounded fetch: a relay that accepts TCP but stalls would
            // otherwise hang Orchestrator::start() (and the whole test)
            // forever — the bare `reqwest::get` had no timeout
            // (problems/2026-07-19-e2e-multi-device-serialization-hang).
            // Guarded by scripts/check-test-timeouts.sh — do not revert
            // to the unbounded call.
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| E2eError::scenario(format!("HTTP client build failed: {e}")))?;
            let mut last_err = String::new();
            let mut fetched = None;
            for attempt in 1..=3u32 {
                match client.get(&key_url).send().await {
                    Ok(resp) => match resp.error_for_status() {
                        Ok(ok) => match ok.bytes().await {
                            Ok(b) => {
                                fetched = Some(b);
                                break;
                            }
                            Err(e) => last_err = format!("read body: {e}"),
                        },
                        Err(e) => last_err = format!("non-2xx: {e}"),
                    },
                    Err(e) => last_err = format!("request: {e}"),
                }
                debug!("OHTTP key fetch attempt {attempt} failed: {last_err}");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            let bytes = fetched.ok_or_else(|| {
                E2eError::scenario(format!(
                    "Failed to fetch {key_url} after 3 attempts: {last_err}"
                ))
            })?;
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
    /// With the default OHTTP topology, `--relay` identifies the application
    /// relay and `VAUCHI_OHTTP_RELAY_URL` separately identifies the outer hop.
    /// Explicit transport-isolation scenarios without an outer hop use the
    /// main listener's HTTP origin instead.
    pub fn primary_cli_relay_url(&self) -> E2eResult<String> {
        if self.ohttp_relay_url().is_some() {
            self.primary_relay_http_url()
        } else {
            let relay_url = self.primary_relay_url()?;
            relay_url
                .strip_prefix("ws://")
                .map(|rest| format!("http://{rest}"))
                .or_else(|| {
                    relay_url
                        .strip_prefix("wss://")
                        .map(|rest| format!("https://{rest}"))
                })
                .ok_or_else(|| E2eError::relay("relay URL must use ws:// or wss://"))
        }
    }

    /// Get the spawned ohttp-relay URL, if `with_ohttp_relay` is active.
    pub fn ohttp_relay_url(&self) -> Option<String> {
        self.ohttp_relay_manager.as_ref().and_then(|m| m.url())
    }

    /// Arm the E2E-only outer-relay duplicate-delivery controller.
    pub async fn arm_ohttp_duplicate_next_forward(&self) -> E2eResult<()> {
        let manager = self.ohttp_relay_manager.as_ref().ok_or_else(|| {
            E2eError::relay("duplicate OHTTP delivery requires an outer OHTTP relay")
        })?;
        manager.arm_duplicate_next_forward().await
    }

    /// Arm the E2E-only outer-relay opaque-forward reorder controller.
    pub async fn arm_ohttp_reorder_next_forward(&self) -> E2eResult<()> {
        let manager = self.ohttp_relay_manager.as_ref().ok_or_else(|| {
            E2eError::relay("reordered OHTTP delivery requires an outer OHTTP relay")
        })?;
        manager.arm_reorder_next_forward().await
    }

    /// Wait until the E2E-only outer relay has held its reordered forward.
    pub async fn wait_for_ohttp_reorder_pending(&self) -> E2eResult<()> {
        let manager = self.ohttp_relay_manager.as_ref().ok_or_else(|| {
            E2eError::relay("reordered OHTTP delivery requires an outer OHTTP relay")
        })?;
        manager.wait_for_reorder_pending().await
    }

    fn cli_ohttp_route_url(&self) -> E2eResult<String> {
        self.ohttp_relay_url()
            .map_or_else(|| self.primary_relay_http_url(), Ok)
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
        let relay_url = self.primary_cli_relay_url()?;

        info!("Adding user '{}' with {} device(s)", name, device_count);

        let mut extra_env = HashMap::new();
        extra_env.insert(
            CLI_OHTTP_RELAY_URL_ENV.to_string(),
            self.cli_ohttp_route_url()?,
        );
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
    /// exercised end-to-end. Unlike ordinary `add_user`, this deliberately
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
        extra_env.insert(CLI_OHTTP_RELAY_URL_ENV.to_string(), ohttp_url);

        let user = UserBuilder::new(&name, relay_url)
            .with_devices(device_count)
            .with_extra_env(extra_env)
            .build()?;

        let user = Arc::new(RwLock::new(user));
        self.users.insert(name, user.clone());

        Ok(user)
    }

    /// Add a user whose primary device is an iOS Simulator controlled by Maestro.
    ///
    /// Does not inject CLI OHTTP overrides; the simulator app connects to the
    /// relay directly. Skips gracefully in tests when no simulator is booted.
    pub fn add_user_with_maestro_ios(
        &mut self,
        name: impl Into<String>,
        simulator_name: impl Into<String>,
    ) -> E2eResult<Arc<RwLock<User>>> {
        let name = name.into();
        let simulator_name = simulator_name.into();
        let relay_url = self.primary_relay_url()?;

        info!(
            "Adding Maestro iOS user '{}' on simulator '{}' (relay: {})",
            name, simulator_name, relay_url
        );

        let mut user = User::with_relay(&name, &relay_url);
        user.add_maestro_ios_device(&simulator_name, &relay_url)?;

        let user = Arc::new(RwLock::new(user));
        self.users.insert(name, user.clone());
        Ok(user)
    }

    /// Add a user whose primary device is an Android Emulator controlled by Maestro.
    ///
    /// Does not inject CLI OHTTP overrides; the emulator app connects to the
    /// relay directly. Skips gracefully in tests when no emulator is connected.
    pub fn add_user_with_maestro_android(
        &mut self,
        name: impl Into<String>,
        emulator_name: impl Into<String>,
    ) -> E2eResult<Arc<RwLock<User>>> {
        let name = name.into();
        let emulator_name = emulator_name.into();
        let relay_url = self.primary_relay_url()?;

        info!(
            "Adding Maestro Android user '{}' on emulator '{}' (relay: {})",
            name, emulator_name, relay_url
        );

        let mut user = User::with_relay(&name, &relay_url);
        user.add_maestro_android_device(&emulator_name, &relay_url)?;

        let user = Arc::new(RwLock::new(user));
        self.users.insert(name, user.clone());
        Ok(user)
    }

    /// Add a user whose primary device is the TUI controlled via PTY automation.
    #[cfg(feature = "tui")]
    pub fn add_user_with_tui(&mut self, name: impl Into<String>) -> E2eResult<Arc<RwLock<User>>> {
        let name = name.into();
        let relay_url = self.primary_cli_relay_url()?;

        info!("Adding TUI user '{}' (relay: {})", name, relay_url);

        let mut user = User::with_relay(&name, &relay_url);
        user.add_tui_device(&relay_url)?;

        let user = Arc::new(RwLock::new(user));
        self.users.insert(name, user.clone());
        Ok(user)
    }

    /// Add a split-OHTTP user whose devices each receive their own test-only
    /// environment values, while the outer relay route remains fixed.
    pub fn add_user_split_ohttp_with_device_envs(
        &mut self,
        name: impl Into<String>,
        device_extra_envs: Vec<HashMap<String, String>>,
    ) -> E2eResult<Arc<RwLock<User>>> {
        if device_extra_envs.is_empty() {
            return Err(E2eError::user(
                "split-OHTTP user requires at least one device",
            ));
        }

        let name = name.into();
        let relay_url = self.primary_relay_http_url()?;
        let ohttp_url = self.ohttp_relay_url().ok_or_else(|| {
            E2eError::relay(
                "add_user_split_ohttp_with_device_envs requires \
                 OrchestratorConfig::with_ohttp_relay",
            )
        })?;
        let mut user = User::with_relay(&name, &relay_url);
        for mut extra_env in device_extra_envs {
            extra_env.insert(CLI_OHTTP_RELAY_URL_ENV.to_string(), ohttp_url.clone());
            user.add_cli_device_with_env(&relay_url, &extra_env)?;
        }

        let user = Arc::new(RwLock::new(user));
        self.users.insert(name, user.clone());
        Ok(user)
    }

    /// Add one CLI device to an existing user through the production-shaped
    /// split relay and OHTTP route.
    pub async fn add_cli_device_split_ohttp(&self, user_name: &str) -> E2eResult<usize> {
        let relay_url = self.primary_relay_http_url()?;
        let ohttp_url = self.ohttp_relay_url().ok_or_else(|| {
            E2eError::relay(
                "add_cli_device_split_ohttp requires OrchestratorConfig::with_ohttp_relay",
            )
        })?;
        let user = self
            .user(user_name)
            .ok_or_else(|| E2eError::user(format!("User '{user_name}' not found")))?;
        let mut extra_env = HashMap::new();
        extra_env.insert(CLI_OHTTP_RELAY_URL_ENV.to_string(), ohttp_url);
        user.write()
            .await
            .add_cli_device_with_env(&relay_url, &extra_env)
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

        // Both sides must start first so each `complete` resumes the exact
        // ephemeral session represented by the QR the peer scanned.
        let qr_a = {
            let user = user_a.read().await;
            user.generate_qr().await?
        };
        let qr_b = {
            let user = user_b.read().await;
            user.generate_qr().await?
        };
        {
            let user = user_b.read().await;
            user.complete_exchange(&qr_a).await?;
        }
        {
            let user = user_a.read().await;
            user.complete_exchange(&qr_b).await?;
        }

        // The exchange completion messages establish the first ratchet
        // direction. Run a complete receive/send round for both peers so
        // callers can immediately publish cards or visibility changes.
        for _ in 0..2 {
            {
                let user = user_a.read().await;
                user.sync_all().await?;
            }
            {
                let user = user_b.read().await;
                user.sync_all().await?;
            }
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
            let contact_names = contacts
                .iter()
                .map(|contact| contact.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(E2eError::assertion(format!(
                "User '{}' primary device has {} contacts [{}], expected {}",
                user_name,
                contacts.len(),
                contact_names,
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

    // @internal
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
