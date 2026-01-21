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
use crate::relay_manager::{RelayConfig, RelayManager};
use crate::user::{User, UserBuilder};

/// Configuration for the orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Relay configuration.
    pub relay_config: RelayConfig,
    /// Number of relays to spawn.
    pub relay_count: usize,
    /// Delay between operations (for observability).
    pub operation_delay: Duration,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            relay_config: RelayConfig::default(),
            relay_count: 1,
            operation_delay: Duration::from_millis(100),
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
    users: HashMap<String, Arc<RwLock<User>>>,
    started: bool,
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
            users: HashMap::new(),
            started: false,
        }
    }

    /// Start the orchestrator (spawn relays).
    pub async fn start(&mut self) -> E2eResult<()> {
        if self.started {
            return Err(E2eError::scenario("Orchestrator already started"));
        }

        info!("Starting orchestrator with {} relay(s)", self.config.relay_count);

        let mut relay_manager = RelayManager::with_config(self.config.relay_config.clone()).await?;
        relay_manager.spawn(self.config.relay_count).await?;

        self.relay_manager = Some(relay_manager);
        self.started = true;

        Ok(())
    }

    /// Stop the orchestrator (cleanup relays).
    pub async fn stop(&mut self) -> E2eResult<()> {
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

    /// Get the primary relay URL.
    pub fn primary_relay_url(&self) -> E2eResult<String> {
        self.relay_manager
            .as_ref()
            .and_then(|rm| rm.relay_url(0))
            .map(|s| s.to_string())
            .ok_or_else(|| E2eError::scenario("No relay available"))
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
    pub fn add_user(&mut self, name: impl Into<String>, device_count: usize) -> E2eResult<Arc<RwLock<User>>> {
        let name = name.into();
        let relay_url = self.primary_relay_url()?;

        info!("Adding user '{}' with {} device(s)", name, device_count);

        let user = UserBuilder::new(&name, relay_url)
            .with_devices(device_count)
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
        let user_a = self.user(user_a_name)
            .ok_or_else(|| E2eError::user(format!("User '{}' not found", user_a_name)))?;
        let user_b = self.user(user_b_name)
            .ok_or_else(|| E2eError::user(format!("User '{}' not found", user_b_name)))?;

        info!("Exchange: {} -> {}", user_a_name, user_b_name);

        // User A generates QR
        let qr = {
            let user = user_a.read().await;
            user.generate_qr().await?
        };

        // User B completes exchange
        {
            let user = user_b.read().await;
            user.complete_exchange(&qr).await?;
        }

        // Both sync
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

    /// Verify that a user has a specific number of contacts on all devices.
    pub async fn verify_contact_count(&self, user_name: &str, expected: usize) -> E2eResult<()> {
        let user = self.user(user_name)
            .ok_or_else(|| E2eError::user(format!("User '{}' not found", user_name)))?;

        let user = user.read().await;

        for i in 0..user.device_count() {
            let contacts = user.list_contacts_on_device(i).await?;

            if contacts.len() != expected {
                return Err(E2eError::assertion(format!(
                    "User '{}' device {} has {} contacts, expected {}",
                    user_name,
                    i,
                    contacts.len(),
                    expected
                )));
            }
        }

        debug!(
            "User '{}' verified: {} contacts on all {} devices",
            user_name,
            expected,
            user.device_count()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_config_default() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.relay_count, 1);
    }

    #[test]
    fn test_orchestrator_new() {
        let orch = Orchestrator::new();
        assert!(!orch.is_running());
        assert_eq!(orch.user_count(), 0);
    }
}
