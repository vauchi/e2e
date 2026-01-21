//! Scenario DSL for E2E testing.
//!
//! Provides a fluent API for defining multi-user, multi-device test scenarios.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::info;

use crate::error::{E2eError, E2eResult};
use crate::relay_manager::{RelayConfig, RelayManager};
use crate::user::{User, UserBuilder};

/// Type alias for async setup/test closures.
pub type AsyncFn<T> = Box<
    dyn for<'a> Fn(&'a ScenarioContext) -> Pin<Box<dyn Future<Output = E2eResult<T>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Configuration for a user in the scenario.
#[derive(Debug, Clone)]
pub struct UserConfig {
    /// User's display name.
    pub name: String,
    /// Number of devices for this user.
    pub device_count: usize,
}

/// A named test within a scenario.
struct NamedTest {
    name: String,
    test_fn: AsyncFn<()>,
}

/// Context available during scenario setup and tests.
pub struct ScenarioContext {
    /// The relay manager.
    pub relays: Arc<RwLock<RelayManager>>,
    /// All users in the scenario.
    users: HashMap<String, Arc<RwLock<User>>>,
    /// Ordered list of user names for iteration.
    user_order: Vec<String>,
}

impl ScenarioContext {
    /// Get a user by name.
    pub fn user(&self, name: &str) -> Option<Arc<RwLock<User>>> {
        self.users.get(name).cloned()
    }

    /// Get all users.
    pub fn users(&self) -> impl Iterator<Item = Arc<RwLock<User>>> + '_ {
        self.user_order.iter().filter_map(|name| self.users.get(name).cloned())
    }

    /// Get user names in order.
    pub fn user_names(&self) -> &[String] {
        &self.user_order
    }

    /// Get the number of users.
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// Get the primary relay URL.
    pub async fn primary_relay_url(&self) -> Option<String> {
        let relays = self.relays.read().await;
        relays.relay_url(0).map(|s| s.to_string())
    }

    /// Get all relay URLs.
    pub async fn all_relay_urls(&self) -> Vec<String> {
        let relays = self.relays.read().await;
        relays.all_urls().iter().map(|s| s.to_string()).collect()
    }

    /// Sync all users' devices.
    pub async fn sync_all(&self) -> E2eResult<()> {
        for user in self.users() {
            let user = user.read().await;
            user.sync_all().await?;
        }
        Ok(())
    }
}

/// Builder for E2E test scenarios.
pub struct Scenario {
    /// Number of relay servers to spawn.
    relay_count: usize,
    /// Relay configuration.
    relay_config: RelayConfig,
    /// User configurations.
    user_configs: Vec<UserConfig>,
    /// Setup function to run before tests.
    setup_fn: Option<AsyncFn<()>>,
    /// Named tests to run.
    tests: Vec<NamedTest>,
}

impl Default for Scenario {
    fn default() -> Self {
        Self::new()
    }
}

impl Scenario {
    /// Create a new scenario builder.
    pub fn new() -> Self {
        Self {
            relay_count: 1,
            relay_config: RelayConfig::default(),
            user_configs: Vec::new(),
            setup_fn: None,
            tests: Vec::new(),
        }
    }

    /// Set the number of relay servers.
    pub fn with_relays(mut self, count: usize) -> Self {
        self.relay_count = count;
        self
    }

    /// Set custom relay configuration.
    pub fn with_relay_config(mut self, config: RelayConfig) -> Self {
        self.relay_config = config;
        self
    }

    /// Add a user with the specified number of devices.
    pub fn with_user(mut self, name: impl Into<String>, device_count: usize) -> Self {
        self.user_configs.push(UserConfig {
            name: name.into(),
            device_count,
        });
        self
    }

    /// Add a setup function to run before tests.
    pub fn setup<F, Fut>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a ScenarioContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = E2eResult<()>> + Send + 'static,
    {
        self.setup_fn = Some(Box::new(move |ctx| Box::pin(f(ctx))));
        self
    }

    /// Add a named test to run.
    pub fn test<F, Fut>(mut self, name: impl Into<String>, f: F) -> Self
    where
        F: for<'a> Fn(&'a ScenarioContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = E2eResult<()>> + Send + 'static,
    {
        self.tests.push(NamedTest {
            name: name.into(),
            test_fn: Box::new(move |ctx| Box::pin(f(ctx))),
        });
        self
    }

    /// Run the scenario.
    pub async fn run(self) -> E2eResult<()> {
        info!(
            "Starting scenario with {} relays and {} users",
            self.relay_count,
            self.user_configs.len()
        );

        // Spawn relay servers
        let mut relay_manager = RelayManager::with_config(self.relay_config).await?;
        relay_manager.spawn(self.relay_count).await?;

        let primary_relay_url = relay_manager
            .relay_url(0)
            .ok_or_else(|| E2eError::scenario("No relay available"))?
            .to_string();

        // Create users with devices
        let mut users = HashMap::new();
        let mut user_order = Vec::new();

        for config in &self.user_configs {
            info!(
                "Creating user '{}' with {} device(s)",
                config.name, config.device_count
            );

            let user = UserBuilder::new(&config.name, &primary_relay_url)
                .with_devices(config.device_count)
                .build()?;

            users.insert(config.name.clone(), Arc::new(RwLock::new(user)));
            user_order.push(config.name.clone());
        }

        // Create context
        let context = ScenarioContext {
            relays: Arc::new(RwLock::new(relay_manager)),
            users,
            user_order,
        };

        // Run setup if provided
        if let Some(setup_fn) = &self.setup_fn {
            info!("Running scenario setup");
            setup_fn(&context).await?;
        }

        // Run tests
        for test in &self.tests {
            info!("Running test: {}", test.name);
            (test.test_fn)(&context).await?;
            info!("Test '{}' passed", test.name);
        }

        // Cleanup
        {
            let mut relays = context.relays.write().await;
            relays.stop_all().await;
        }

        info!("Scenario completed successfully");
        Ok(())
    }
}

/// Prebuilt scenario templates for common test patterns.
pub mod templates {
    use super::*;

    /// Create the standard five-user scenario from the plan.
    ///
    /// Users:
    /// - Alice: 3 devices
    /// - Bob: 2 devices
    /// - Carol: 1 device
    /// - Dave: 1 device
    /// - Eve: 3 devices
    pub fn five_users() -> Scenario {
        Scenario::new()
            .with_relays(2)
            .with_user("Alice", 3)
            .with_user("Bob", 2)
            .with_user("Carol", 1)
            .with_user("Dave", 1)
            .with_user("Eve", 3)
    }

    /// Create a simple two-user scenario for basic testing.
    pub fn two_users() -> Scenario {
        Scenario::new()
            .with_relays(1)
            .with_user("Alice", 1)
            .with_user("Bob", 1)
    }

    /// Create a multi-device sync test scenario.
    pub fn multi_device_sync() -> Scenario {
        Scenario::new()
            .with_relays(1)
            .with_user("Alice", 3)
            .with_user("Bob", 2)
    }

    /// Create a relay failover test scenario.
    pub fn relay_failover() -> Scenario {
        Scenario::new()
            .with_relays(2)
            .with_user("Alice", 1)
            .with_user("Bob", 1)
    }
}

/// Helper functions for common test operations.
pub mod helpers {
    use super::*;

    /// Set up identities for all users on their primary devices.
    pub async fn create_all_identities(ctx: &ScenarioContext) -> E2eResult<()> {
        for user in ctx.users() {
            let user = user.read().await;
            user.create_identity().await?;
        }
        Ok(())
    }

    /// Link all devices for all users.
    pub async fn link_all_devices(ctx: &ScenarioContext) -> E2eResult<()> {
        for user in ctx.users() {
            let user = user.read().await;
            user.link_devices().await?;
        }
        Ok(())
    }

    /// Standard setup: create identities and link devices.
    pub async fn standard_setup(ctx: &ScenarioContext) -> E2eResult<()> {
        create_all_identities(ctx).await?;
        link_all_devices(ctx).await?;
        Ok(())
    }

    /// Exchange between all users (n*(n-1)/2 exchanges).
    pub async fn exchange_all_users(ctx: &ScenarioContext) -> E2eResult<()> {
        let names: Vec<String> = ctx.user_names().to_vec();

        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                let user_a = ctx.user(&names[i]).unwrap();
                let user_b = ctx.user(&names[j]).unwrap();

                let user_a_guard = user_a.read().await;
                let user_b_guard = user_b.read().await;

                info!("Exchanging: {} <-> {}", user_a_guard.name(), user_b_guard.name());

                // User A generates QR, User B completes
                let qr = user_a_guard.generate_qr().await?;
                user_b_guard.complete_exchange(&qr).await?;
            }
        }

        // Sync all after exchanges
        ctx.sync_all().await?;

        Ok(())
    }

    /// Verify that a user has the expected number of contacts on all devices.
    pub async fn verify_contact_count(
        ctx: &ScenarioContext,
        user_name: &str,
        expected_count: usize,
    ) -> E2eResult<()> {
        let user = ctx
            .user(user_name)
            .ok_or_else(|| E2eError::assertion(format!("User '{}' not found", user_name)))?;

        let user = user.read().await;

        for i in 0..user.device_count() {
            let contacts = user.list_contacts_on_device(i).await?;
            if contacts.len() != expected_count {
                return Err(E2eError::assertion(format!(
                    "User '{}' device {} has {} contacts, expected {}",
                    user_name,
                    i,
                    contacts.len(),
                    expected_count
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_builder() {
        let scenario = Scenario::new()
            .with_relays(2)
            .with_user("Alice", 3)
            .with_user("Bob", 2);

        assert_eq!(scenario.relay_count, 2);
        assert_eq!(scenario.user_configs.len(), 2);
        assert_eq!(scenario.user_configs[0].name, "Alice");
        assert_eq!(scenario.user_configs[0].device_count, 3);
    }

    #[test]
    fn test_five_users_template() {
        let scenario = templates::five_users();

        assert_eq!(scenario.relay_count, 2);
        assert_eq!(scenario.user_configs.len(), 5);

        // Verify device counts
        assert_eq!(scenario.user_configs[0].device_count, 3); // Alice
        assert_eq!(scenario.user_configs[1].device_count, 2); // Bob
        assert_eq!(scenario.user_configs[2].device_count, 1); // Carol
        assert_eq!(scenario.user_configs[3].device_count, 1); // Dave
        assert_eq!(scenario.user_configs[4].device_count, 3); // Eve
    }
}
