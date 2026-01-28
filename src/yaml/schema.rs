// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! YAML Scenario Schema Definitions
//!
//! Defines the structure of YAML scenario files for E2E testing.
//! Scenarios can be executed both automatically and manually.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// A complete E2E test scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    /// Unique name of the scenario.
    pub name: String,

    /// Optional description of what this scenario tests.
    #[serde(default)]
    pub description: Option<String>,

    /// Reference to Gherkin feature file for traceability.
    #[serde(default)]
    pub feature: Option<String>,

    /// Tags for filtering (e.g., ["smoke", "exchange"]).
    #[serde(default)]
    pub tags: Vec<String>,

    /// Scenario timeout.
    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub timeout: Duration,

    /// Participants in this scenario.
    #[serde(default)]
    pub participants: HashMap<String, Participant>,

    /// Relay configuration for this scenario.
    #[serde(default)]
    pub relay_config: Option<RelayConfig>,

    /// Network conditions to simulate.
    #[serde(default)]
    pub network_conditions: HashMap<String, NetworkCondition>,

    /// The test steps to execute.
    pub steps: Vec<Step>,

    /// Manual instructions for human testers.
    #[serde(default)]
    pub manual_instructions: Vec<String>,

    /// Platforms this scenario applies to.
    #[serde(default)]
    pub platforms: Option<Vec<Platform>>,

    /// If true, this scenario can only be run manually.
    #[serde(default)]
    pub manual_only: bool,
}

fn default_timeout() -> Duration {
    Duration::from_secs(60)
}

/// A participant in a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    /// Number of devices for this participant.
    #[serde(default = "default_device_count")]
    pub devices: usize,

    /// Platform requirement (any, cli, ios, android, desktop, tui).
    #[serde(default)]
    pub platform: Platform,
}

fn default_device_count() -> usize {
    1
}

/// Supported platforms for testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Any,
    Cli,
    Ios,
    Android,
    Desktop,
    Tui,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::Any => write!(f, "any"),
            Platform::Cli => write!(f, "cli"),
            Platform::Ios => write!(f, "ios"),
            Platform::Android => write!(f, "android"),
            Platform::Desktop => write!(f, "desktop"),
            Platform::Tui => write!(f, "tui"),
        }
    }
}

/// Relay configuration for a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    /// Number of relays to spawn.
    #[serde(default = "default_relay_count")]
    pub count: usize,

    /// Base port for relays.
    #[serde(default)]
    pub base_port: Option<u16>,

    /// Storage backend (memory or sqlite).
    #[serde(default)]
    pub storage_backend: Option<String>,
}

fn default_relay_count() -> usize {
    1
}

/// Network condition simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkCondition {
    /// Whether internet is available.
    #[serde(default = "default_internet")]
    pub internet: bool,

    /// Network mode for simulation.
    #[serde(default)]
    pub mode: NetworkMode,

    /// Packet drop rate (0.0 to 1.0) for flaky mode.
    #[serde(default)]
    pub drop_rate: f32,

    /// Added latency in milliseconds.
    #[serde(default)]
    pub latency_ms: u32,
}

fn default_internet() -> bool {
    true
}

/// Network simulation modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Normal,
    Flaky,
    Offline,
}

/// A single step in a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    /// Execute an action.
    Action(ActionStep),

    /// Make an assertion.
    Assertion(AssertionStep),

    /// Wait for a duration.
    Wait(WaitStep),

    /// Run steps in parallel.
    Parallel(ParallelStep),

    /// Conditional execution.
    If(ConditionalStep),
}

/// An action to perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionStep {
    /// The action to perform.
    pub action: Action,

    /// The actor(s) performing the action.
    pub actor: ActorRef,

    /// Parameters for the action.
    #[serde(default)]
    pub params: HashMap<String, serde_yaml::Value>,

    /// Variable to store output in.
    #[serde(default)]
    pub output: Option<String>,

    /// Expected error (if action should fail).
    #[serde(default)]
    pub expect_error: Option<String>,

    /// Number of retries on failure.
    #[serde(default)]
    pub retries: u32,

    /// Step-specific timeout.
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
}

/// Available actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    // Identity management
    CreateIdentity,
    ExportIdentity,
    ImportIdentity,

    // Exchange
    GenerateQr,
    CompleteExchange,
    Exchange,
    MutualExchange,

    // Device linking
    StartDeviceLink,
    JoinIdentity,
    CompleteDeviceLink,
    FinishDeviceJoin,
    LinkDevices,

    // Sync
    Sync,
    SyncAll,
    SyncDevice,

    // Card management
    GetCard,
    AddField,
    EditField,
    RemoveField,
    EditName,
    UpdateCard,

    // Contact management
    ListContacts,
    GetContact,

    // Network simulation
    SetNetwork,

    // App lifecycle
    BackgroundApp,
    ForegroundApp,
    KillApp,
    LaunchApp,
    DeviceRestart,

    // Proximity
    StartProximityVerification,
    VerifyProximity,

    // Relay control
    StopRelay,
    RestartRelay,

    // Utilities
    Wait,
    Log,
}

/// Reference to an actor (user or device).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActorRef {
    /// Single actor reference (e.g., "alice" or "alice.device1").
    Single(String),

    /// Multiple actors (e.g., ["alice", "bob"]).
    Multiple(Vec<String>),
}

impl ActorRef {
    /// Get all actor references as a vector.
    pub fn actors(&self) -> Vec<&str> {
        match self {
            ActorRef::Single(s) => vec![s.as_str()],
            ActorRef::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }

    /// Parse an actor reference into user and optional device index.
    ///
    /// Formats:
    /// - "alice" -> ("alice", None)
    /// - "alice.device0" -> ("alice", Some(0))
    /// - "alice.primary" -> ("alice", None) - primary means default device
    pub fn parse(actor: &str) -> (&str, Option<usize>) {
        if let Some((user, device)) = actor.split_once('.') {
            // "primary" is an alias for the default/first device
            if device == "primary" {
                return (user, None);
            }
            if let Some(idx_str) = device.strip_prefix("device") {
                if let Ok(idx) = idx_str.parse() {
                    return (user, Some(idx));
                }
            }
        }
        (actor, None)
    }
}

/// An assertion to verify.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertionStep {
    /// The assertion to make.
    pub assertion: Assertion,

    /// The actor(s) to check.
    #[serde(default)]
    pub actor: Option<ActorRef>,

    /// Parameters for the assertion.
    #[serde(default)]
    pub params: HashMap<String, serde_yaml::Value>,

    /// Assertion-specific timeout.
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
}

/// Available assertions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assertion {
    HasContact,
    ContactCount,
    ContactsMatch,
    ContactsSynced,
    HasIdentity,
    IdentityLoaded,
    CardHasField,
    CardFieldEquals,
    CardsConverged,
    SessionRestored,
    ProximityVerified,
    DeviceCount,
    RelayConnected,
    SyncSucceeded,
    SyncFailed,
}

/// Wait for a duration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitStep {
    /// Duration to wait.
    #[serde(with = "humantime_serde")]
    pub duration: Duration,

    /// Optional reason for waiting.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Run steps in parallel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelStep {
    /// Steps to run in parallel.
    pub steps: Vec<Step>,

    /// Whether all steps must succeed.
    #[serde(default = "default_true")]
    pub all_must_succeed: bool,
}

fn default_true() -> bool {
    true
}

/// Conditional step execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalStep {
    /// Condition to check.
    pub condition: String,

    /// Steps to run if condition is true.
    pub then: Vec<Step>,

    /// Steps to run if condition is false.
    #[serde(default)]
    pub otherwise: Vec<Step>,
}

/// Result of executing a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    /// Scenario name.
    pub name: String,

    /// Whether the scenario passed.
    pub passed: bool,

    /// Duration of execution.
    pub duration: Duration,

    /// Results for each step.
    pub steps: Vec<StepResult>,

    /// Error message if failed.
    pub error: Option<String>,

    /// Output variables collected during execution.
    pub outputs: HashMap<String, String>,
}

/// Result of executing a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step description.
    pub description: String,

    /// Whether the step passed.
    pub passed: bool,

    /// Duration of this step.
    pub duration: Duration,

    /// Error message if failed.
    pub error: Option<String>,

    /// Output value if any.
    pub output: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_actor_ref() {
        assert_eq!(ActorRef::parse("alice"), ("alice", None));
        assert_eq!(ActorRef::parse("alice.device0"), ("alice", Some(0)));
        assert_eq!(ActorRef::parse("bob.device2"), ("bob", Some(2)));
        assert_eq!(ActorRef::parse("alice.primary"), ("alice", None));
    }

    #[test]
    fn test_deserialize_simple_scenario() {
        let yaml = r#"
name: "Simple Exchange"
feature: "contact_exchange.feature"
tags: ["smoke", "exchange"]
timeout: 30s

participants:
  alice: { devices: 1 }
  bob: { devices: 1 }

steps:
  - type: action
    action: create_identity
    actor: alice
    params:
      name: "Alice"

  - type: action
    action: create_identity
    actor: bob
    params:
      name: "Bob"

  - type: action
    action: generate_qr
    actor: alice
    output: qr_data

  - type: action
    action: complete_exchange
    actor: bob
    params:
      qr: "${qr_data}"

  - type: assertion
    assertion: contact_count
    actor: [alice, bob]
    params:
      count: 1

manual_instructions:
  - "Alice: Create identity 'Alice'"
  - "Bob: Create identity 'Bob'"
  - "Alice: Show QR code"
  - "Bob: Scan Alice's QR code"
  - "Verify: Both have 1 contact"
"#;

        let scenario: Scenario = serde_yaml::from_str(yaml).expect("Failed to parse YAML");
        assert_eq!(scenario.name, "Simple Exchange");
        assert_eq!(scenario.tags, vec!["smoke", "exchange"]);
        assert_eq!(scenario.timeout, Duration::from_secs(30));
        assert_eq!(scenario.participants.len(), 2);
        assert_eq!(scenario.steps.len(), 5);
        assert_eq!(scenario.manual_instructions.len(), 5);
    }

    #[test]
    fn test_deserialize_network_conditions() {
        let yaml = r#"
name: "Offline Test"
participants:
  alice: { devices: 1 }

network_conditions:
  alice:
    internet: false
    mode: offline

steps:
  - type: action
    action: set_network
    actor: alice
    params:
      internet: false
"#;

        let scenario: Scenario = serde_yaml::from_str(yaml).expect("Failed to parse YAML");
        let alice_network = scenario.network_conditions.get("alice").unwrap();
        assert!(!alice_network.internet);
        assert_eq!(alice_network.mode, NetworkMode::Offline);
    }
}
