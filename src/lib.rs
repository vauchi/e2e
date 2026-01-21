//! End-to-end testing infrastructure for Vauchi
//!
//! This crate provides a comprehensive testing framework for simulating
//! multi-user, multi-device scenarios across different platforms.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                     E2E Test Orchestrator                           │
//! ├─────────────────────────────────────────────────────────────────────┤
//! │  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐          │
//! │  │   Relay A    │    │   Relay B    │    │  Test Clock  │          │
//! │  │  :8080       │    │  :8081       │    │  (simulated) │          │
//! │  └──────────────┘    └──────────────┘    └──────────────┘          │
//! │                                                                     │
//! │  ┌──────────────────────────────────────────────────────────────┐  │
//! │  │              User Instance Manager                           │  │
//! │  │    ┌─────────┬─────────┬─────────┬─────────┬─────────┐      │  │
//! │  │    │ Alice   │ Bob     │ Carol   │ Dave    │ Eve     │      │  │
//! │  │    │ 3 devs  │ 2 devs  │ 2 devs  │ 1 dev   │ 3 devs  │      │  │
//! │  │    └─────────┴─────────┴─────────┴─────────┴─────────┘      │  │
//! │  └──────────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use vauchi_e2e_tests::prelude::*;
//!
//! #[tokio::test]
//! async fn test_five_user_exchange() {
//!     Scenario::new()
//!         .with_relays(2)
//!         .with_user("Alice", 3)
//!         .with_user("Bob", 2)
//!         .with_user("Carol", 1)
//!         .with_user("Dave", 1)
//!         .with_user("Eve", 3)
//!         .setup(|ctx| async move {
//!             for user in ctx.users() {
//!                 user.primary_device().create_identity(&user.name()).await?;
//!             }
//!             Ok(())
//!         })
//!         .test("exchange_all", |ctx| async move {
//!             // Test implementation
//!             Ok(())
//!         })
//!         .run()
//!         .await
//!         .unwrap();
//! }
//! ```

pub mod device;
pub mod error;
pub mod orchestrator;
pub mod relay_manager;
pub mod scenario;
pub mod user;

pub mod prelude {
    //! Re-exports commonly used types for convenience.
    pub use crate::device::{CliDevice, Contact, ContactCard, Device, DeviceType};
    pub use crate::error::{E2eError, E2eResult};
    pub use crate::orchestrator::{Orchestrator, OrchestratorConfig};
    pub use crate::relay_manager::{RelayConfig, RelayInstance, RelayManager};
    pub use crate::scenario::{Scenario, ScenarioContext};
    pub use crate::user::User;
}
