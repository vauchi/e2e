// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Relay server management for E2E tests.
//!
//! Spawns and manages isolated relay server instances for testing.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::time::timeout;
use tracing::{debug, info};

use crate::error::{E2eError, E2eResult};

/// Reserve an available port pair (relay + metrics) by binding to port 0.
///
/// Asks the OS to assign a free port, then immediately releases it.
/// The metrics port (relay + 1000) is also probed.
///
/// There is a small TOCTOU window between releasing the listeners and
/// the relay binary binding — mitigated by the retry loop in `spawn_one`.
pub fn find_available_port() -> E2eResult<u16> {
    for _ in 0..100 {
        // Ask the OS for a free port (bind to :0).
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| E2eError::relay(format!("Failed to bind ephemeral port: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| E2eError::relay(format!("Failed to get local addr: {e}")))?
            .port();

        // Also verify the metrics port (port + 1000) is free.
        // Skip ports that would overflow u16 when adding 1000.
        let Some(metrics_port) = port.checked_add(1000) else {
            continue;
        };
        let metrics_ok = TcpListener::bind(format!("127.0.0.1:{metrics_port}")).is_ok();

        // Drop both listeners to release the ports right before relay spawn.
        drop(listener);

        if metrics_ok {
            return Ok(port);
        }
    }

    Err(E2eError::relay(
        "Could not find available port pair for relay server",
    ))
}

/// Timeout for relay startup.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// A running relay server instance.
pub struct RelayInstance {
    /// The relay's WebSocket URL.
    pub url: String,
    /// The relay's HTTP API URL (v2 endpoints, OHTTP, health).
    pub http_url: String,
    /// The relay's port.
    pub port: u16,
    /// The metrics endpoint port.
    pub metrics_port: u16,
    /// The child process handle.
    process: Option<Child>,
    /// Index of this relay in the manager (reserved for future use).
    #[allow(dead_code)]
    index: usize,
}

impl RelayInstance {
    /// Returns the WebSocket URL for this relay.
    pub fn ws_url(&self) -> &str {
        &self.url
    }

    /// Returns the HTTP health check URL.
    pub fn health_url(&self) -> String {
        format!("http://127.0.0.1:{}/health", self.port)
    }

    /// Returns the metrics URL.
    pub fn metrics_url(&self) -> String {
        format!("http://127.0.0.1:{}/metrics", self.metrics_port)
    }

    /// Returns the HTTP base URL (for v2 API / OHTTP gateway).
    ///
    /// The v2 HTTP API runs on the metrics port (`RELAY_METRICS_ADDR`),
    /// not the main WebSocket port.
    pub fn http_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.metrics_port)
    }

    /// Check if the relay is running.
    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }
}

impl Drop for RelayInstance {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // Kill and wait for exit so the port is fully released.
            // start_kill() alone returns immediately — the process may
            // still hold the port when the next test tries to bind it.
            let _ = process.start_kill();
            // Block briefly for the process to exit. In async context
            // kill_on_drop(true) handles cleanup, but in sync Drop we
            // need a wait to avoid port leaks between tests.
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                if let Ok(rt) = rt {
                    let _ = rt.block_on(async {
                        tokio::time::timeout(std::time::Duration::from_secs(5), process.wait())
                            .await
                    });
                }
            })
            .join()
            .ok();
        }
    }
}

/// Configuration for spawning relay instances.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Base port for relay servers (0 = auto-allocate).
    /// When set to 0, ports are dynamically allocated to avoid conflicts
    /// when tests run in parallel.
    pub base_port: u16,
    /// Path to the relay binary (auto-detected if None).
    pub binary_path: Option<PathBuf>,
    /// Storage backend ("memory" or "sqlite").
    pub storage_backend: String,
    /// Blob TTL in seconds (default: 3600).
    pub blob_ttl_secs: u64,
    /// Idle timeout in seconds (default: 60).
    pub idle_timeout: u64,
    /// Rate limit per minute.  Set high for e2e tests (5000) because
    /// multi-device sync tests compress many user-operations into seconds.
    /// Production relay uses a lower value.
    pub rate_limit: u32,
    /// Enable the HTTP API v2 (required for OHTTP).
    pub http_api_enabled: bool,
    /// Enable the OHTTP gateway (requires `http_api_enabled`).
    pub ohttp_enabled: bool,
    /// OHTTP key rotation interval in hours (default: 24).
    pub ohttp_key_rotation_hours: u64,
    /// Override rotation interval in seconds (for key rotation tests).
    pub ohttp_key_rotation_secs: Option<u64>,
    /// Version policy: minimum required client compat version (0 = no enforcement).
    pub version_min: Option<u16>,
    /// Version policy: version at which the relay warns clients to upgrade.
    pub version_warn: Option<u16>,
    /// Version policy: grace period days after min_version is raised.
    pub version_grace_days: Option<u16>,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            // Use 0 to indicate dynamic port allocation
            base_port: 0,
            binary_path: None,
            storage_backend: "memory".to_string(),
            blob_ttl_secs: 3600,
            idle_timeout: 60,
            rate_limit: 100_000,
            http_api_enabled: true,
            ohttp_enabled: true,
            ohttp_key_rotation_hours: 24,
            ohttp_key_rotation_secs: None,
            version_min: None,
            version_warn: None,
            version_grace_days: None,
        }
    }
}

/// Manages multiple relay server instances.
pub struct RelayManager {
    config: RelayConfig,
    relays: Vec<RelayInstance>,
    binary_path: PathBuf,
}

impl RelayManager {
    /// Create a new relay manager with default configuration.
    pub async fn new() -> E2eResult<Self> {
        Self::with_config(RelayConfig::default()).await
    }

    /// Create a new relay manager with custom configuration.
    pub async fn with_config(config: RelayConfig) -> E2eResult<Self> {
        let binary_path = if let Some(ref path) = config.binary_path {
            path.clone()
        } else {
            Self::find_relay_binary()?
        };

        debug!("Using relay binary at: {}", binary_path.display());

        Ok(Self {
            config,
            relays: Vec::new(),
            binary_path,
        })
    }

    /// Find the relay binary in the workspace.
    fn find_relay_binary() -> E2eResult<PathBuf> {
        // Try E2E_BIN_DIR first (SHA-cached binaries from build-binaries.sh)
        if let Ok(dir) = std::env::var("E2E_BIN_DIR") {
            let path = PathBuf::from(&dir).join("vauchi-relay");
            if path.exists() {
                return Ok(path);
            }
        }

        // Try release binary first
        let release_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/release/vauchi-relay");
        if release_path.exists() {
            return Ok(release_path);
        }

        // Try debug binary
        let debug_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/vauchi-relay");
        if debug_path.exists() {
            return Ok(debug_path);
        }

        Err(E2eError::relay(
            "Relay binary not found. Please run `just build-relay` first.",
        ))
    }

    /// Spawn a specified number of relay instances.
    pub async fn spawn(&mut self, count: usize) -> E2eResult<()> {
        for i in 0..count {
            self.spawn_one(i).await?;
        }
        Ok(())
    }

    /// Spawn a single relay instance at the given index.
    ///
    /// Retries with a new port if the relay crashes on startup (SIGABRT from
    /// port TOCTOU races when nextest runs parallel test binaries).
    async fn spawn_one(&mut self, index: usize) -> E2eResult<()> {
        let max_retries = 3;
        for attempt in 0..max_retries {
            match self.try_spawn_one(index).await {
                Ok(()) => return Ok(()),
                Err(e) if attempt + 1 < max_retries => {
                    info!(
                        "Relay {} spawn failed (attempt {}), retrying: {}",
                        index,
                        attempt + 1,
                        e
                    );
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Attempt to spawn a single relay instance.
    async fn try_spawn_one(&mut self, index: usize) -> E2eResult<()> {
        // Allocate port dynamically if base_port is 0
        let port = if self.config.base_port == 0 {
            find_available_port()?
        } else {
            self.config.base_port + index as u16
        };
        // Metrics port is relay port + 1000
        let metrics_port = port + 1000;

        info!("Spawning relay {} on port {}", index, port);

        let mut env_vars: HashMap<String, String> = HashMap::new();
        env_vars.insert(
            "RELAY_LISTEN_ADDR".to_string(),
            format!("127.0.0.1:{}", port),
        );
        env_vars.insert(
            "RELAY_METRICS_ADDR".to_string(),
            format!("127.0.0.1:{}", metrics_port),
        );
        env_vars.insert(
            "RELAY_STORAGE_BACKEND".to_string(),
            self.config.storage_backend.clone(),
        );
        env_vars.insert(
            "RELAY_BLOB_TTL_SECS".to_string(),
            self.config.blob_ttl_secs.to_string(),
        );
        env_vars.insert(
            "RELAY_IDLE_TIMEOUT".to_string(),
            self.config.idle_timeout.to_string(),
        );
        env_vars.insert(
            "RELAY_RATE_LIMIT".to_string(),
            self.config.rate_limit.to_string(),
        );
        // Use the same rate limit for the OHTTP exchange path.
        env_vars.insert(
            "RELAY_OHTTP_EXCHANGE_RATE_LIMIT".to_string(),
            self.config.rate_limit.to_string(),
        );
        // Disable Noise encryption requirement for E2E tests.
        // The CLI doesn't support Noise NK — it uses plaintext v1 connections.
        env_vars.insert(
            "RELAY_REQUIRE_NOISE_ENCRYPTION".to_string(),
            "false".to_string(),
        );
        // Enable HTTP API v2 and OHTTP gateway when configured (defaults to true).
        if self.config.http_api_enabled {
            env_vars.insert("RELAY_HTTP_API_ENABLED".to_string(), "true".to_string());
        }
        if self.config.ohttp_enabled {
            env_vars.insert("RELAY_OHTTP_ENABLED".to_string(), "true".to_string());
            env_vars.insert(
                "RELAY_OHTTP_KEY_ROTATION_HOURS".to_string(),
                self.config.ohttp_key_rotation_hours.to_string(),
            );
            if let Some(secs) = self.config.ohttp_key_rotation_secs {
                env_vars.insert(
                    "RELAY_OHTTP_KEY_ROTATION_SECS".to_string(),
                    secs.to_string(),
                );
            }
        }
        env_vars.insert("RUST_LOG".to_string(), "warn".to_string());
        self.add_version_policy_env_vars(&mut env_vars);

        let mut cmd = Command::new(&self.binary_path);
        cmd.envs(env_vars)
            // Redirect both stdout and stderr to null to prevent buffer filling.
            // When pipes are not consumed, the buffer fills (~64KB) and the relay
            // process blocks on write(), becoming unresponsive under heavy load.
            // We use TCP health checks instead of monitoring stderr for readiness.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| E2eError::relay(format!("Failed to spawn relay {}: {}", index, e)))?;

        // Verify the relay is actually listening and serving requests
        let url = format!("ws://127.0.0.1:{}", port);
        let http_url = format!("http://127.0.0.1:{}", metrics_port);
        self.wait_for_health(port, index, &mut child).await?;

        let instance = RelayInstance {
            url,
            http_url,
            port,
            metrics_port,
            process: Some(child),
            index,
        };

        self.relays.push(instance);
        info!("Relay {} started successfully on port {}", index, port);

        Ok(())
    }

    /// Wait for a relay to become healthy.
    ///
    /// Polls both the main port (`/health`) and the HTTP API / metrics port.
    /// Both must respond before the relay is considered ready.  Previously only
    /// the main port was checked, so a relay whose metrics port failed to bind
    /// (port conflict) passed the health check while the CLI — which connects
    /// to the metrics port — got "Connection refused".
    async fn wait_for_health(&self, port: u16, index: usize, child: &mut Child) -> E2eResult<()> {
        let health_url = format!("http://127.0.0.1:{}/health", port);
        let metrics_port = port + 1000;
        let metrics_url = format!("http://127.0.0.1:{}/health", metrics_port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| E2eError::relay(format!("Failed to create HTTP client: {}", e)))?;

        let check_health = async {
            for attempt in 0..60 {
                // Check process liveness first — fail fast if relay crashed
                if let Some(exit_status) = child.try_wait().map_err(|e| {
                    E2eError::relay(format!("Failed to check relay {} status: {}", index, e))
                })? {
                    return Err(E2eError::relay(format!(
                        "Relay {} exited during startup with status: {}",
                        index, exit_status
                    )));
                }

                // Check main port
                let main_ok = match client.get(&health_url).send().await {
                    Ok(resp) if resp.status().is_success() => true,
                    Ok(resp) => {
                        debug!(
                            "Relay {} returned {} (attempt {})",
                            index,
                            resp.status(),
                            attempt + 1
                        );
                        false
                    }
                    Err(_) => false,
                };

                // Check metrics / HTTP API port (where the CLI connects)
                let metrics_ok = main_ok
                    && matches!(
                        client.get(&metrics_url).send().await,
                        Ok(resp) if resp.status().is_success()
                    );

                if main_ok && metrics_ok {
                    debug!("Relay {} healthy (attempt {})", index, attempt + 1);
                    return Ok(());
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(E2eError::timeout(format!(
                "Relay {} failed to start within timeout (health: {}, metrics: {})",
                index, health_url, metrics_url
            )))
        };

        timeout(STARTUP_TIMEOUT, check_health)
            .await
            .map_err(|_| E2eError::timeout(format!("Relay {} startup timed out", index)))?
    }

    /// Get the WebSocket URL for a relay by index.
    pub fn relay_url(&self, index: usize) -> Option<&str> {
        self.relays.get(index).map(|r| r.url.as_str())
    }

    /// Get the HTTP API URL for a relay by index.
    ///
    /// The v2 endpoints (OHTTP, exchange, sync) are served on the
    /// HTTP/metrics port, not the WebSocket port.
    pub fn relay_http_url(&self, index: usize) -> Option<&str> {
        self.relays.get(index).map(|r| r.http_url.as_str())
    }

    /// Get a relay instance by index.
    pub fn relay(&self, index: usize) -> Option<&RelayInstance> {
        self.relays.get(index)
    }

    /// Get all relay URLs.
    pub fn all_urls(&self) -> Vec<&str> {
        self.relays.iter().map(|r| r.url.as_str()).collect()
    }

    /// Get the number of running relays.
    pub fn count(&self) -> usize {
        self.relays.len()
    }

    /// Add version policy env vars if configured.
    fn add_version_policy_env_vars(&self, env_vars: &mut HashMap<String, String>) {
        if let Some(min) = self.config.version_min {
            env_vars.insert("RELAY_VERSION_MIN".to_string(), min.to_string());
        }
        if let Some(warn) = self.config.version_warn {
            env_vars.insert("RELAY_VERSION_WARN".to_string(), warn.to_string());
        }
        if let Some(grace) = self.config.version_grace_days {
            env_vars.insert("RELAY_VERSION_GRACE_DAYS".to_string(), grace.to_string());
        }
    }

    /// Stop a specific relay.
    pub async fn stop_relay(&mut self, index: usize) -> E2eResult<()> {
        if let Some(relay) = self.relays.get_mut(index) {
            if let Some(mut process) = relay.process.take() {
                info!("Stopping relay {}", index);
                process.kill().await.map_err(|e| {
                    E2eError::relay(format!("Failed to stop relay {}: {}", index, e))
                })?;
            }
            Ok(())
        } else {
            Err(E2eError::relay(format!("Relay {} not found", index)))
        }
    }

    /// Restart a specific relay.
    pub async fn restart_relay(&mut self, index: usize) -> E2eResult<()> {
        // Get the port from existing relay (we need to reuse the same port)
        let (port, metrics_port) = if let Some(relay) = self.relays.get(index) {
            (relay.port, relay.metrics_port)
        } else {
            return Err(E2eError::relay(format!("Relay {} not found", index)));
        };

        // Stop if running
        if let Some(relay) = self.relays.get_mut(index)
            && let Some(mut process) = relay.process.take()
        {
            info!("Stopping relay {} for restart", index);
            let _ = process.kill().await;
        }

        // Small delay before restart
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut env_vars: HashMap<String, String> = HashMap::new();
        env_vars.insert(
            "RELAY_LISTEN_ADDR".to_string(),
            format!("127.0.0.1:{}", port),
        );
        env_vars.insert(
            "RELAY_METRICS_ADDR".to_string(),
            format!("127.0.0.1:{}", metrics_port),
        );
        env_vars.insert(
            "RELAY_STORAGE_BACKEND".to_string(),
            self.config.storage_backend.clone(),
        );
        env_vars.insert(
            "RELAY_BLOB_TTL_SECS".to_string(),
            self.config.blob_ttl_secs.to_string(),
        );
        env_vars.insert(
            "RELAY_IDLE_TIMEOUT".to_string(),
            self.config.idle_timeout.to_string(),
        );
        env_vars.insert(
            "RELAY_RATE_LIMIT".to_string(),
            self.config.rate_limit.to_string(),
        );
        env_vars.insert(
            "RELAY_OHTTP_EXCHANGE_RATE_LIMIT".to_string(),
            self.config.rate_limit.to_string(),
        );
        env_vars.insert(
            "RELAY_REQUIRE_NOISE_ENCRYPTION".to_string(),
            "false".to_string(),
        );
        if self.config.http_api_enabled {
            env_vars.insert("RELAY_HTTP_API_ENABLED".to_string(), "true".to_string());
        }
        if self.config.ohttp_enabled {
            env_vars.insert("RELAY_OHTTP_ENABLED".to_string(), "true".to_string());
            env_vars.insert(
                "RELAY_OHTTP_KEY_ROTATION_HOURS".to_string(),
                self.config.ohttp_key_rotation_hours.to_string(),
            );
            if let Some(secs) = self.config.ohttp_key_rotation_secs {
                env_vars.insert(
                    "RELAY_OHTTP_KEY_ROTATION_SECS".to_string(),
                    secs.to_string(),
                );
            }
        }
        env_vars.insert("RUST_LOG".to_string(), "warn".to_string());
        self.add_version_policy_env_vars(&mut env_vars);

        let mut cmd = Command::new(&self.binary_path);
        cmd.envs(env_vars)
            // Use null for both to prevent buffer filling (same as spawn_one)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| E2eError::relay(format!("Failed to restart relay {}: {}", index, e)))?;

        // Wait for health
        self.wait_for_health(port, index, &mut child).await?;

        // Update the instance
        if let Some(relay) = self.relays.get_mut(index) {
            relay.process = Some(child);
            info!("Relay {} restarted successfully", index);
        }

        Ok(())
    }

    /// Stop all relays.
    pub async fn stop_all(&mut self) {
        info!("Stopping all {} relays", self.relays.len());
        for relay in &mut self.relays {
            if let Some(mut process) = relay.process.take() {
                let _ = process.kill().await;
            }
        }
    }
}

impl Drop for RelayManager {
    fn drop(&mut self) {
        // Kill all relay processes and wait for exit so ports are released
        // before the next test starts. Without waiting, TCP TIME_WAIT can
        // cause port conflicts in subsequent tests.
        let mut children: Vec<_> = self
            .relays
            .iter_mut()
            .filter_map(|r| r.process.take())
            .collect();
        for child in &mut children {
            let _ = child.start_kill();
        }
        if !children.is_empty() {
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                if let Ok(rt) = rt {
                    rt.block_on(async {
                        for child in &mut children {
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                child.wait(),
                            )
                            .await;
                        }
                    });
                }
            })
            .join()
            .ok();
        }
    }
}

// INLINE_TEST_REQUIRED: tests exercise private function find_available_port
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_config_default() {
        let config = RelayConfig::default();
        // base_port 0 means dynamic allocation
        assert_eq!(config.base_port, 0);
        assert_eq!(config.storage_backend, "memory");
    }

    #[test]
    fn test_find_available_port() {
        let port1 = find_available_port().expect("Should find port");
        let port2 = find_available_port().expect("Should find port");

        // OS-assigned ports must be distinct
        assert_ne!(port1, port2, "consecutive ports must differ");

        // Ports must be non-privileged (>1024) and in valid u16 range
        assert!(port1 > 1024, "port {port1} should be non-privileged");
        assert!(port2 > 1024, "port {port2} should be non-privileged");

        // Both the relay port and its metrics port (port+1000) must be bindable
        // right after allocation — this is the whole point of the reservation.
        for port in [port1, port2] {
            let listener = TcpListener::bind(format!("127.0.0.1:{port}"));
            assert!(
                listener.is_ok(),
                "port {port} should be bindable after allocation"
            );
            let metrics = TcpListener::bind(format!("127.0.0.1:{}", port + 1000));
            assert!(
                metrics.is_ok(),
                "metrics port {} should be bindable",
                port + 1000
            );
        }
    }
}
