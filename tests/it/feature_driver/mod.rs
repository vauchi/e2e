// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use async_trait::async_trait;
use cucumber::{World, given, then, when, writer::Stats as _};
use vauchi_e2e_tests::device::ExchangeLinkFeedback;

mod app;
mod contract;
mod escrow;
mod model;
mod model_tests;
mod presentation;

#[async_trait]
trait FeatureDriver: Send {
    async fn add_user(&mut self, name: &str) -> Result<()>;
    async fn exchange_link(&mut self, initiator: &str, responder: &str) -> Result<()>;
    async fn open_link(&mut self, owner: &str, url: &str) -> Result<ExchangeLinkFeedback>;
    async fn contact_names(&mut self, owner: &str) -> Result<Vec<String>>;
    async fn stop(&mut self) -> Result<()>;
    // The CLI and TUI cannot enter a link code until their Core bump
    // (vauchi/private#727), so only the contract driver answers these.
    async fn share_link(&mut self, _initiator: &str) -> Result<()> {
        anyhow::bail!("this driver cannot enter a link code")
    }
    async fn open_shared_link_with_wrong_code(&mut self, _responder: &str) -> Result<()> {
        anyhow::bail!("this driver cannot enter a link code")
    }
    async fn waiting_for_exchange(&mut self, _owner: &str) -> Result<bool> {
        anyhow::bail!("this driver cannot enter a link code")
    }
}

#[derive(Clone, Copy)]
pub(super) enum Backend {
    Contract,
    Cli,
    #[cfg(feature = "tui")]
    Tui,
}

#[derive(Default, World)]
struct FeatureWorld {
    driver: Option<Box<dyn FeatureDriver>>,
    link_feedback: Option<(String, ExchangeLinkFeedback)>,
}

impl std::fmt::Debug for FeatureWorld {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FeatureWorld")
            .finish_non_exhaustive()
    }
}

impl FeatureWorld {
    fn driver(&mut self) -> &mut dyn FeatureDriver {
        self.driver
            .as_deref_mut()
            .expect("scenario driver initialized")
    }
}

#[given(expr = "a user {string}")]
async fn a_user(world: &mut FeatureWorld, name: String) {
    world.driver().add_user(&name).await.expect("create user");
}

#[when(expr = "{string} shares an exchange link with {string}")]
async fn share_link(world: &mut FeatureWorld, initiator: String, responder: String) {
    world
        .driver()
        .exchange_link(&initiator, &responder)
        .await
        .expect("exchange through driver");
}

#[when(expr = "{string} shares an exchange link")]
async fn share_link_alone(world: &mut FeatureWorld, initiator: String) {
    world
        .driver()
        .share_link(&initiator)
        .await
        .expect("share a link through the driver");
}

#[when(expr = "{string} opens the exchange link with a wrong code")]
async fn open_link_with_wrong_code(world: &mut FeatureWorld, responder: String) {
    world
        .driver()
        .open_shared_link_with_wrong_code(&responder)
        .await
        .expect("open the shared link with a wrong code");
}

#[then(expr = "{string} is still waiting for the exchange")]
async fn still_waiting(world: &mut FeatureWorld, owner: String) {
    assert!(
        world
            .driver()
            .waiting_for_exchange(&owner)
            .await
            .expect("observe the exchange screen"),
        "{owner} should still be waiting, not told the exchange failed"
    );
}

#[then(expr = "{string} has {string} as a contact")]
async fn has_contact(world: &mut FeatureWorld, owner: String, peer: String) {
    assert_eq!(
        world
            .driver()
            .contact_names(&owner)
            .await
            .expect("observe contacts"),
        vec![peer]
    );
}

#[when(expr = "{string} opens a malformed exchange link")]
async fn open_malformed_link(world: &mut FeatureWorld, owner: String) {
    let feedback = world
        .driver()
        .open_link(&owner, "https://vauchi.app/x#pk=invalid&n=invalid")
        .await
        .expect("observe link feedback through the real interface");
    world.link_feedback = Some((owner, feedback));
}

#[then(expr = "{string} is told the exchange link is invalid")]
async fn invalid_link_feedback(world: &mut FeatureWorld, owner: String) {
    assert_eq!(
        world.link_feedback,
        Some((owner, ExchangeLinkFeedback::InvalidLink))
    );
}

#[then(expr = "{string} has no contacts")]
async fn has_no_contacts(world: &mut FeatureWorld, owner: String) {
    assert_eq!(
        world
            .driver()
            .contact_names(&owner)
            .await
            .expect("observe contacts"),
        Vec::<String>::new()
    );
}

pub(super) async fn run(backend: Backend) {
    let feature = std::env::var_os("VAUCHI_FEATURES_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../features"))
        .join("link_exchange.feature");
    assert!(
        feature.is_file(),
        "missing canonical feature: {}",
        feature.display()
    );
    let writer = FeatureWorld::cucumber()
        .with_default_cli()
        .max_concurrent_scenarios(1)
        .before(move |_, _, _, world| {
            Box::pin(async move {
                world.driver = Some(match backend {
                    Backend::Contract => {
                        Box::new(contract::ContractDriver::start().expect("start contract driver"))
                    }
                    Backend::Cli => Box::new(
                        app::AppDriver::start(app::AppKind::Cli)
                            .await
                            .expect("start CLI driver"),
                    ),
                    #[cfg(feature = "tui")]
                    Backend::Tui => Box::new(
                        app::AppDriver::start(app::AppKind::Tui)
                            .await
                            .expect("start TUI driver"),
                    ),
                });
            })
        })
        .after(|_, _, _, _, world| {
            Box::pin(async move {
                if let Some(world) = world
                    && let Some(mut driver) = world.driver.take()
                {
                    driver.stop().await.expect("stop scenario driver");
                }
            })
        })
        .filter_run(feature, move |_, _, scenario| {
            matches!(
                scenario.name.as_str(),
                "Two users exchange contact cards using a link"
                    | "A malformed exchange link is rejected without adding contacts"
                    | "Users exchange contact cards after a malformed link is rejected"
            ) || (matches!(backend, Backend::Contract)
                && scenario.name == "A wrong code makes no contact")
        })
        .await;
    let stats = writer.scenarios_stats();
    assert!(!writer.execution_has_failed(), "feature execution failed");
    let expected = if matches!(backend, Backend::Contract) {
        4
    } else {
        3
    };
    assert_eq!(
        (stats.passed, stats.failed, stats.skipped),
        (expected, 0, 0)
    );
}

// @scenario: link_exchange :: Two users exchange contact cards using a link
// @scenario: link_exchange :: A malformed exchange link is rejected without adding contacts
// @scenario: link_exchange :: Users exchange contact cards after a malformed link is rejected
#[tokio::test]
async fn integration_shared_driver_contract_link_exchange() {
    run(Backend::Contract).await;
}
