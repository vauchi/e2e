// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OHTTP relay (forwarding proxy) management for E2E tests.
//!
//! Spawns and manages the `vauchi-ohttp-relay` binary that sits between clients
//! and the vauchi-relay gateway, stripping client IPs from OHTTP requests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::time::timeout;
use tracing::{debug, info};

use crate::error::{E2eError, E2eResult};
use crate::relay_manager::find_available_port;

/// Timeout for vauchi-ohttp-relay startup.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Configuration for the OHTTP relay proxy.
#[derive(Debug, Clone)]
pub struct OhttpRelayConfig {
    /// Path to the vauchi-ohttp-relay binary (auto-detected if None).
    pub binary_path: Option<PathBuf>,
    /// Per-IP rate limit (requests/sec). 0 disables rate limiting.
    pub rate_limit_per_sec: u32,
    /// Request timeout in seconds.
    pub request_timeout_secs: u64,
    /// TTL for caching upstream `/v2/ohttp-key` responses (seconds).
    /// Set to 0 to disable caching. Default: 300.
    pub key_cache_ttl_secs: u64,
}

impl Default for OhttpRelayConfig {
    fn default() -> Self {
        Self {
            binary_path: None,
            rate_limit_per_sec: 100,
            request_timeout_secs: 30,
            key_cache_ttl_secs: 300,
        }
    }
}

/// A running OHTTP relay proxy instance.
pub struct OhttpRelayInstance {
    /// The port this relay listens on.
    pub port: u16,
    /// The upstream gateway URL this relay forwards to.
    pub gateway_url: String,
    /// The child process handle.
    process: Option<Child>,
}

impl OhttpRelayInstance {
    /// Returns the base URL clients should use for OHTTP requests.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Returns the health check URL.
    pub fn health_url(&self) -> String {
        format!("http://127.0.0.1:{}/health", self.port)
    }
}

impl Drop for OhttpRelayInstance {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // Kill and wait for exit so the port is fully released.
            // start_kill() alone returns immediately — the process may
            // still hold the port when the next test tries to bind it.
            let _ = process.start_kill();
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

/// Manages OHTTP relay proxy instances for E2E tests.
pub struct OhttpRelayManager {
    config: OhttpRelayConfig,
    binary_path: PathBuf,
    instance: Option<OhttpRelayInstance>,
}

impl OhttpRelayManager {
    /// Create a new OHTTP relay manager.
    pub fn new(config: OhttpRelayConfig) -> E2eResult<Self> {
        let binary_path = if let Some(ref path) = config.binary_path {
            path.clone()
        } else {
            Self::find_binary()?
        };

        debug!(
            "Using vauchi-ohttp-relay binary at: {}",
            binary_path.display()
        );

        Ok(Self {
            config,
            binary_path,
            instance: None,
        })
    }

    /// Find the vauchi-ohttp-relay binary in the workspace.
    fn find_binary() -> E2eResult<PathBuf> {
        if let Ok(dir) = std::env::var("E2E_BIN_DIR") {
            let path = PathBuf::from(&dir).join("vauchi-ohttp-relay");
            if path.exists() {
                return Ok(path);
            }
        }

        let release_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/release/vauchi-ohttp-relay");
        if release_path.exists() {
            return Ok(release_path);
        }

        let debug_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/vauchi-ohttp-relay");
        if debug_path.exists() {
            return Ok(debug_path);
        }

        // ohttp-relay has its own Cargo workspace (not in the root workspace),
        // so its binary lands in ohttp-relay/target/ instead of target/.
        let ohttp_own_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../ohttp-relay/target/debug/vauchi-ohttp-relay");
        if ohttp_own_path.exists() {
            return Ok(ohttp_own_path);
        }

        Err(E2eError::relay(
            "vauchi-ohttp-relay binary not found. Please run `just build vauchi-ohttp-relay` first.",
        ))
    }

    /// Spawn the OHTTP relay, forwarding to the given gateway URL.
    pub async fn spawn(&mut self, gateway_url: &str) -> E2eResult<()> {
        let port = find_available_port()?;

        info!(
            "Spawning vauchi-ohttp-relay on port {} → {}",
            port, gateway_url
        );

        let mut env_vars: HashMap<String, String> = HashMap::new();
        env_vars.insert(
            "OHTTP_RELAY_LISTEN_ADDR".to_string(),
            format!("127.0.0.1:{}", port),
        );
        env_vars.insert(
            "OHTTP_RELAY_GATEWAY_URL".to_string(),
            gateway_url.to_string(),
        );
        env_vars.insert(
            "OHTTP_RELAY_RATE_LIMIT_PER_SEC".to_string(),
            self.config.rate_limit_per_sec.to_string(),
        );
        env_vars.insert(
            "OHTTP_RELAY_REQUEST_TIMEOUT_SECS".to_string(),
            self.config.request_timeout_secs.to_string(),
        );
        env_vars.insert(
            "OHTTP_RELAY_KEY_CACHE_TTL_SECS".to_string(),
            self.config.key_cache_ttl_secs.to_string(),
        );
        env_vars.insert("RUST_LOG".to_string(), "warn".to_string());

        let mut cmd = Command::new(&self.binary_path);
        cmd.envs(env_vars)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| E2eError::relay(format!("Failed to spawn vauchi-ohttp-relay: {}", e)))?;

        self.wait_for_health(port, &mut child).await?;

        self.instance = Some(OhttpRelayInstance {
            port,
            gateway_url: gateway_url.to_string(),
            process: Some(child),
        });

        info!("vauchi-ohttp-relay started on port {}", port);
        Ok(())
    }

    /// Wait for the OHTTP relay to become healthy.
    async fn wait_for_health(&self, port: u16, child: &mut Child) -> E2eResult<()> {
        let health_url = format!("http://127.0.0.1:{}/health", port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| E2eError::relay(format!("Failed to create HTTP client: {}", e)))?;

        let check_health = async {
            for attempt in 0..60 {
                if let Some(exit_status) = child.try_wait().map_err(|e| {
                    E2eError::relay(format!("Failed to check vauchi-ohttp-relay status: {}", e))
                })? {
                    return Err(E2eError::relay(format!(
                        "vauchi-ohttp-relay exited during startup with status: {}",
                        exit_status
                    )));
                }

                match client.get(&health_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        debug!("vauchi-ohttp-relay healthy (attempt {})", attempt + 1);
                        return Ok(());
                    }
                    _ => {}
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(E2eError::timeout(
                "vauchi-ohttp-relay failed to start within timeout",
            ))
        };

        timeout(STARTUP_TIMEOUT, check_health)
            .await
            .map_err(|_| E2eError::timeout("vauchi-ohttp-relay startup timed out"))?
    }

    /// Get the running instance.
    pub fn instance(&self) -> Option<&OhttpRelayInstance> {
        self.instance.as_ref()
    }

    /// Get the URL clients should use (the vauchi-ohttp-relay's listen address).
    pub fn url(&self) -> Option<String> {
        self.instance.as_ref().map(|i| i.url())
    }

    /// Stop the OHTTP relay.
    pub async fn stop(&mut self) {
        if let Some(mut instance) = self.instance.take()
            && let Some(mut process) = instance.process.take()
        {
            info!("Stopping vauchi-ohttp-relay");
            let _ = process.kill().await;
        }
    }
}

impl Drop for OhttpRelayManager {
    fn drop(&mut self) {
        if let Some(mut instance) = self.instance.take()
            && let Some(mut process) = instance.process.take()
        {
            let _ = process.start_kill();
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
