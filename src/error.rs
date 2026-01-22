//! Error types for E2E testing infrastructure.

use std::io;
use thiserror::Error;

/// Result type alias for E2E operations.
pub type E2eResult<T> = Result<T, E2eError>;

/// Errors that can occur during E2E testing.
#[derive(Error, Debug)]
pub enum E2eError {
    /// Failed to spawn or manage a relay server.
    #[error("Relay error: {0}")]
    Relay(String),

    /// Failed to execute CLI command.
    #[error("CLI execution error: {0}")]
    CliExecution(String),

    /// CLI command returned an error.
    #[error("CLI command failed: {command}\nstderr: {stderr}")]
    CliCommand { command: String, stderr: String },

    /// Failed to parse CLI output.
    #[error("Failed to parse CLI output: {0}")]
    ParseOutput(String),

    /// Device operation failed.
    #[error("Device error: {0}")]
    Device(String),

    /// User operation failed.
    #[error("User error: {0}")]
    User(String),

    /// Scenario setup or execution failed.
    #[error("Scenario error: {0}")]
    Scenario(String),

    /// Failed to load a YAML scenario.
    #[error("Scenario load error: {0}")]
    ScenarioLoad(String),

    /// Invalid actor reference in scenario.
    #[error("Invalid actor: {0}")]
    InvalidActor(String),

    /// Invalid step in scenario.
    #[error("Invalid step: {0}")]
    InvalidStep(String),

    /// Device type not supported.
    #[error("Device not supported: {0}")]
    DeviceNotSupported(String),

    /// Timeout waiting for an operation.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Assertion failed during test.
    #[error("Assertion failed: {0}")]
    Assertion(String),

    /// Assertion failed with details.
    #[error("Assertion failed: {0}")]
    AssertionFailed(String),

    /// YAML parsing error.
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Generic error.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl E2eError {
    /// Create a relay error.
    pub fn relay(msg: impl Into<String>) -> Self {
        Self::Relay(msg.into())
    }

    /// Create a CLI execution error.
    pub fn cli_execution(msg: impl Into<String>) -> Self {
        Self::CliExecution(msg.into())
    }

    /// Create a parse output error.
    pub fn parse_output(msg: impl Into<String>) -> Self {
        Self::ParseOutput(msg.into())
    }

    /// Create a device error.
    pub fn device(msg: impl Into<String>) -> Self {
        Self::Device(msg.into())
    }

    /// Create a user error.
    pub fn user(msg: impl Into<String>) -> Self {
        Self::User(msg.into())
    }

    /// Create a scenario error.
    pub fn scenario(msg: impl Into<String>) -> Self {
        Self::Scenario(msg.into())
    }

    /// Create a timeout error.
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::Timeout(msg.into())
    }

    /// Create an assertion error.
    pub fn assertion(msg: impl Into<String>) -> Self {
        Self::Assertion(msg.into())
    }
}
