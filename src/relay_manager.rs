//! Relay server management for E2E tests.
//!
//! Spawns and manages isolated relay server instances for testing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::error::{E2eError, E2eResult};

/// Default base port for relay servers.
const DEFAULT_BASE_PORT: u16 = 18080;

/// Default base port for metrics endpoints.
const DEFAULT_METRICS_BASE_PORT: u16 = 19080;

/// Timeout for relay startup.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// A running relay server instance.
pub struct RelayInstance {
    /// The relay's WebSocket URL.
    pub url: String,
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

    /// Check if the relay is running.
    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }
}

impl Drop for RelayInstance {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // Try to kill the process gracefully
            let _ = process.start_kill();
        }
    }
}

/// Configuration for spawning relay instances.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Base port for relay servers (default: 18080).
    pub base_port: u16,
    /// Base port for metrics endpoints (default: 19080).
    pub metrics_base_port: u16,
    /// Path to the relay binary (auto-detected if None).
    pub binary_path: Option<PathBuf>,
    /// Storage backend ("memory" or "sqlite").
    pub storage_backend: String,
    /// Blob TTL in seconds (default: 3600).
    pub blob_ttl_secs: u64,
    /// Idle timeout in seconds (default: 60).
    pub idle_timeout: u64,
    /// Rate limit per minute (default: 1000 for testing).
    pub rate_limit: u32,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            base_port: DEFAULT_BASE_PORT,
            metrics_base_port: DEFAULT_METRICS_BASE_PORT,
            binary_path: None,
            storage_backend: "memory".to_string(),
            blob_ttl_secs: 3600,
            idle_timeout: 60,
            rate_limit: 1000,
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
        // Try release binary first
        let release_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/release/vauchi-relay");
        if release_path.exists() {
            return Ok(release_path);
        }

        // Try debug binary
        let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/debug/vauchi-relay");
        if debug_path.exists() {
            return Ok(debug_path);
        }

        // Fall back to cargo run
        Err(E2eError::relay(
            "Relay binary not found. Please run `cargo build -p vauchi-relay` first.",
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
    async fn spawn_one(&mut self, index: usize) -> E2eResult<()> {
        let port = self.config.base_port + index as u16;
        let metrics_port = self.config.metrics_base_port + index as u16;

        info!("Spawning relay {} on port {}", index, port);

        let mut env_vars: HashMap<String, String> = HashMap::new();
        env_vars.insert("RELAY_LISTEN_ADDR".to_string(), format!("127.0.0.1:{}", port));
        env_vars.insert("RELAY_METRICS_ADDR".to_string(), format!("127.0.0.1:{}", metrics_port));
        env_vars.insert("RELAY_STORAGE_BACKEND".to_string(), self.config.storage_backend.clone());
        env_vars.insert("RELAY_BLOB_TTL_SECS".to_string(), self.config.blob_ttl_secs.to_string());
        env_vars.insert("RELAY_IDLE_TIMEOUT".to_string(), self.config.idle_timeout.to_string());
        env_vars.insert("RELAY_RATE_LIMIT".to_string(), self.config.rate_limit.to_string());
        env_vars.insert("RUST_LOG".to_string(), "warn".to_string());

        let mut cmd = Command::new(&self.binary_path);
        cmd.envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            E2eError::relay(format!("Failed to spawn relay {}: {}", index, e))
        })?;

        // Wait for the relay to start by monitoring stderr for the "listening" message
        if let Some(stderr) = child.stderr.take() {
            let mut reader = BufReader::new(stderr).lines();

            let wait_for_ready = async {
                while let Ok(Some(line)) = reader.next_line().await {
                    debug!("Relay {}: {}", index, line);
                    // Check for various indicators that the server is ready
                    if line.contains("listening") || line.contains("Listening") || line.contains("started") {
                        return Ok(());
                    }
                    if line.contains("error") || line.contains("Error") || line.contains("failed") {
                        return Err(E2eError::relay(format!("Relay {} failed to start: {}", index, line)));
                    }
                }
                Err(E2eError::relay(format!("Relay {} stderr closed unexpectedly", index)))
            };

            // Wait with timeout, but don't fail if we timeout - just do a health check
            match timeout(Duration::from_secs(5), wait_for_ready).await {
                Ok(Ok(())) => {
                    debug!("Relay {} reported ready", index);
                }
                Ok(Err(e)) => {
                    warn!("Relay {} startup warning: {}", index, e);
                }
                Err(_) => {
                    debug!("Relay {} startup timeout, checking health...", index);
                }
            }
        }

        // Verify the relay is actually listening by doing a health check
        let url = format!("ws://127.0.0.1:{}", port);
        self.wait_for_health(port, index).await?;

        let instance = RelayInstance {
            url,
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
    async fn wait_for_health(&self, port: u16, index: usize) -> E2eResult<()> {
        let health_url = format!("http://127.0.0.1:{}/health", port);

        let check_health = async {
            for attempt in 0..60 {
                // Try TCP connection first
                match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
                    Ok(_) => {
                        debug!("Relay {} accepting connections (attempt {})", index, attempt + 1);
                        return Ok(());
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            Err(E2eError::timeout(format!(
                "Relay {} failed to start within timeout (health check at {})",
                index, health_url
            )))
        };

        timeout(STARTUP_TIMEOUT, check_health)
            .await
            .map_err(|_| E2eError::timeout(format!("Relay {} startup timed out", index)))?
    }

    /// Get the URL for a relay by index.
    pub fn relay_url(&self, index: usize) -> Option<&str> {
        self.relays.get(index).map(|r| r.url.as_str())
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
        // Stop if running
        if let Some(relay) = self.relays.get_mut(index) {
            if let Some(mut process) = relay.process.take() {
                info!("Stopping relay {} for restart", index);
                let _ = process.kill().await;
            }
        }

        // Small delay before restart
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Respawn at the same index
        let port = self.config.base_port + index as u16;
        let metrics_port = self.config.metrics_base_port + index as u16;

        let mut env_vars: HashMap<String, String> = HashMap::new();
        env_vars.insert("RELAY_LISTEN_ADDR".to_string(), format!("127.0.0.1:{}", port));
        env_vars.insert("RELAY_METRICS_ADDR".to_string(), format!("127.0.0.1:{}", metrics_port));
        env_vars.insert("RELAY_STORAGE_BACKEND".to_string(), self.config.storage_backend.clone());
        env_vars.insert("RELAY_BLOB_TTL_SECS".to_string(), self.config.blob_ttl_secs.to_string());
        env_vars.insert("RELAY_IDLE_TIMEOUT".to_string(), self.config.idle_timeout.to_string());
        env_vars.insert("RELAY_RATE_LIMIT".to_string(), self.config.rate_limit.to_string());
        env_vars.insert("RUST_LOG".to_string(), "warn".to_string());

        let mut cmd = Command::new(&self.binary_path);
        cmd.envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = cmd.spawn().map_err(|e| {
            E2eError::relay(format!("Failed to restart relay {}: {}", index, e))
        })?;

        // Wait for health
        self.wait_for_health(port, index).await?;

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
        // Synchronous cleanup - just start the kill, don't wait
        for relay in &mut self.relays {
            if let Some(mut process) = relay.process.take() {
                let _ = process.start_kill();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_config_default() {
        let config = RelayConfig::default();
        assert_eq!(config.base_port, DEFAULT_BASE_PORT);
        assert_eq!(config.storage_backend, "memory");
    }
}
