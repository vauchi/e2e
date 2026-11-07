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

const LINK_CODE_LABEL: &str = "6-digit code";
const WAITING_TITLE: &str = "Waiting for Response";
// Enough scheduled wakeups for both handshake deposits and the card deposits
// to reach the relay, well inside the sender's 300 s window.
const WRONG_CODE_WAKEUP_ROUNDS: usize = 16;

struct SharedLink {
    initiator: String,
    url: String,
    code: String,
}

pub(super) struct ContractDriver {
    relay: EscrowAdapter,
    actors: BTreeMap<String, Actor>,
    shared: Option<SharedLink>,
}

impl ContractDriver {
    pub(super) fn start() -> Result<Self> {
        Ok(Self {
            relay: EscrowAdapter::start()?,
            actors: BTreeMap::new(),
            shared: None,
        })
    }

    fn actor(&mut self, name: &str) -> Result<&mut Actor> {
        self.actors.get_mut(name).context("unknown scenario actor")
    }

    fn share(&mut self, initiator: &str) -> Result<SharedLink> {
        let alice = self.actor(initiator)?;
        alice.activate("Exchange")?;
        alice.activate("Link")?;
        let code = alice.shown_code()?;
        alice.activate("Share")?;
        Ok(SharedLink {
            initiator: initiator.into(),
            url: alice.shared_url()?,
            code,
        })
    }

    fn open_with_code(&mut self, responder: &str, url: &str, code: &str) -> Result<()> {
        let bob = self.actor(responder)?;
        bob.dispatch(Event::DeepLinkOpened { uri: url.into() })?;
        bob.activate("Accept Exchange")?;
        bob.type_into(LINK_CODE_LABEL, code)?;
        bob.activate("Continue")
    }
}

fn a_wrong_code(code: &str) -> String {
    code.chars()
        .enumerate()
        .map(|(index, digit)| match (index, digit.to_digit(10)) {
            (0, Some(value)) => char::from_digit((value + 1) % 10, 10).unwrap_or(digit),
            _ => digit,
        })
        .collect()
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
        let completed = [initiator, responder]
            .map(|name| self.actors.get(name).map(Actor::celebrations))
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .context("unknown exchange actor")?;
        let link = self.share(initiator)?;
        self.open_with_code(responder, &link.url, &link.code)?;
        for _ in 0..64 {
            for (name, completed) in [initiator, responder].into_iter().zip(&completed) {
                let actor = self
                    .actors
                    .get_mut(name)
                    .context("unknown exchange actor")?;
                if actor.celebrations() == *completed {
                    actor.wake()?;
                }
            }
            if [initiator, responder]
                .into_iter()
                .zip(&completed)
                .all(|(name, completed)| self.actors[name].celebrations() == completed + 1)
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

    async fn share_link(&mut self, initiator: &str) -> Result<()> {
        self.shared = Some(self.share(initiator)?);
        Ok(())
    }

    async fn open_shared_link_with_wrong_code(&mut self, responder: &str) -> Result<()> {
        let link = self.shared.take().context("no link was shared")?;
        ensure!(
            link.initiator != responder,
            "exchange requires distinct actors"
        );
        self.open_with_code(responder, &link.url, &a_wrong_code(&link.code))?;
        for _ in 0..WRONG_CODE_WAKEUP_ROUNDS {
            for name in [link.initiator.as_str(), responder] {
                self.actor(name)?.wake_if_scheduled()?;
            }
        }
        ensure!(
            self.relay.requests() > 0,
            "exchange bypassed the controlled transport"
        );
        Ok(())
    }

    async fn waiting_for_exchange(&mut self, owner: &str) -> Result<bool> {
        Ok(self.actor(owner)?.title().as_deref() == Some(WAITING_TITLE))
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
