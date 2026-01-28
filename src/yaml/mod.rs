// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! YAML Scenario System
//!
//! Provides a unified format for defining E2E test scenarios that can be
//! executed both automatically and manually.
//!
//! # Example
//!
//! ```yaml
//! name: "Contact Exchange via QR"
//! feature: "contact_exchange.feature"
//! tags: ["smoke", "exchange"]
//! timeout: 60s
//!
//! participants:
//!   alice: { devices: 1 }
//!   bob: { devices: 1 }
//!
//! steps:
//!   - type: action
//!     action: create_identity
//!     actor: alice
//!     params:
//!       name: "Alice Smith"
//!
//!   - type: action
//!     action: generate_qr
//!     actor: alice
//!     output: qr_data
//!
//!   - type: action
//!     action: complete_exchange
//!     actor: bob
//!     params:
//!       qr: "${qr_data}"
//!
//!   - type: assertion
//!     assertion: contact_count
//!     actor: [alice, bob]
//!     params:
//!       count: 1
//!
//! manual_instructions:
//!   - "Alice: Create identity 'Alice Smith'"
//!   - "Alice: Exchange > Show QR"
//!   - "Bob: Create identity 'Bob Jones'"
//!   - "Bob: Exchange > Scan Alice's QR"
//!   - "Verify: Both see each other in Contacts"
//! ```

pub mod executor;
pub mod loader;
pub mod schema;

pub use executor::ScenarioExecutor;
pub use loader::{ScenarioInfo, ScenarioLoader};
pub use schema::{
    Action, ActorRef, Assertion, NetworkCondition, NetworkMode, Participant, Platform, RelayConfig,
    Scenario, ScenarioResult, Step, StepResult,
};
