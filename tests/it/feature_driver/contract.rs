// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::FeatureDriver;
use super::{escrow::EscrowAdapter, presentation::Actor};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use vauchi_app::ui::AppEngine;
use vauchi_core::clock::FakeClock;
use vauchi_core::rng::OsSecureRng;
use vauchi_core::{Command, Event, Vauchi, VauchiConfig};

pub(super) struct ContractDriver {
    relay: EscrowAdapter,
    actors: BTreeMap<String, Actor>,
}

impl ContractDriver {
    pub(super) fn start() -> Result<Self> {
        Ok(Self {
            relay: EscrowAdapter::start()?,
            actors: BTreeMap::new(),
        })
    }
}

#[async_trait]
impl FeatureDriver for ContractDriver {
    async fn add_user(&mut self, name: &str) -> Result<()> {
        ensure!(!self.actors.contains_key(name), "duplicate scenario actor");
        let storage = tempfile::tempdir()?;
        let mut config = VauchiConfig::with_storage_path(storage.path().join("vauchi.db"))
            .with_relay_url(self.relay.url.clone());
        config.ohttp.allow_direct = true;
        config.content_updates = vauchi_core::settings::ContentUpdates::Off;
        let clock = Arc::new(FakeClock::new(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        ));
        let mut core = Vauchi::new_with(config, clock.clone(), OsSecureRng::shared(), None)?;
        core.create_identity(name)?;
        self.actors.insert(
            name.into(),
            Actor::new(AppEngine::new(core), clock, storage)?,
        );
        Ok(())
    }
    async fn exchange_link(&mut self, initiator: &str, responder: &str) -> Result<()> {
        ensure!(initiator != responder, "exchange requires distinct actors");
        let alice = self
            .actors
            .get_mut(initiator)
            .context("unknown initiator")?;
        alice.activate("Exchange")?;
        alice.activate("Link")?;
        alice.activate("Share")?;
        let url = alice.shared_url()?;
        let bob = self
            .actors
            .get_mut(responder)
            .context("unknown responder")?;
        bob.dispatch(Event::DeepLinkOpened { uri: url })?;
        bob.activate("Accept Exchange")?;
        for _ in 0..64 {
            for name in [initiator, responder] {
                let actor = self
                    .actors
                    .get_mut(name)
                    .context("unknown exchange actor")?;
                if actor.celebrations() == 0 {
                    actor.wake()?;
                }
            }
            if [initiator, responder]
                .into_iter()
                .all(|name| self.actors[name].celebrations() == 1)
            {
                ensure!(
                    self.relay.requests() > 0,
                    "exchange bypassed the controlled transport"
                );
                return Ok(());
            }
        }
        bail!("both actors did not present validated exchange success within 64 scheduled wakeups")
    }
    async fn contact_names(&mut self, owner: &str) -> Result<Vec<String>> {
        let actor = self
            .actors
            .get_mut(owner)
            .context("unknown scenario actor")?;
        actor.activate("Contacts")?;
        Ok(actor.contact_names())
    }

    async fn open_link(&mut self, owner: &str, url: &str) -> Result<super::ExchangeLinkFeedback> {
        let actor = self
            .actors
            .get_mut(owner)
            .context("unknown scenario actor")?;
        let start = actor.emitted.len();
        actor.dispatch(Event::DeepLinkOpened { uri: url.into() })?;
        let alert = actor.emitted[start..]
            .iter()
            .find_map(|command| match command {
                Command::PresentAlert { alert } => Some(alert),
                _ => None,
            })
            .context("Core did not present link feedback")?;
        if alert.title == "Invalid Link" && alert.message == "This link cannot be opened in Vauchi."
        {
            return Ok(super::ExchangeLinkFeedback::InvalidLink);
        }
        Ok(super::ExchangeLinkFeedback::Other(format!(
            "{}: {}",
            alert.title, alert.message
        )))
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

// @internal
#[tokio::test]
async fn integration_shared_driver_missing_alert_cannot_report_rejection() {
    let mut driver = ContractDriver::start().expect("controlled driver");
    driver.add_user("Bob").await.expect("Bob");
    driver
        .actors
        .get_mut("Bob")
        .expect("Bob actor")
        .drop_presentation_updates = true;
    let error = driver
        .open_link("Bob", "https://vauchi.app/x#pk=invalid&n=invalid")
        .await
        .expect_err("missing alert cannot report an invalid-link observation");
    assert_eq!(error.to_string(), "Core did not present link feedback");
}

// @internal
#[tokio::test]
async fn integration_shared_driver_dropped_presentation_cannot_complete() {
    let mut driver = ContractDriver::start().expect("controlled driver");
    driver.add_user("Alice").await.expect("Alice");
    driver.add_user("Bob").await.expect("Bob");
    driver
        .actors
        .get_mut("Alice")
        .expect("Alice actor")
        .drop_presentation_updates = true;
    let error = driver
        .exchange_link("Alice", "Bob")
        .await
        .expect_err("dropped presentation must not be healed by hidden snapshots");
    assert_eq!(
        error.to_string(),
        "no presented enabled action named \"Exchange\""
    );
    assert_eq!(driver.relay.requests(), 0);
}

// @internal
#[tokio::test]
async fn integration_shared_driver_without_delivery_never_adds_contacts() {
    let mut driver = ContractDriver::start().expect("controlled driver");
    driver.add_user("Alice").await.expect("Alice");
    driver.add_user("Bob").await.expect("Bob");
    driver.relay.block_delivery();
    let error = driver
        .exchange_link("Alice", "Bob")
        .await
        .expect_err("delivery is required");
    assert_eq!(
        error.to_string(),
        "both actors did not present validated exchange success within 64 scheduled wakeups"
    );
    assert!(
        driver.relay.requests() > 0,
        "failure must reach the transport boundary"
    );
    assert_eq!(
        driver.contact_names("Alice").await.expect("Alice contacts"),
        Vec::<String>::new()
    );
    assert_eq!(
        driver.contact_names("Bob").await.expect("Bob contacts"),
        Vec::<String>::new()
    );
}
