// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::FeatureDriver;
use anyhow::{Context, Result};
use async_trait::async_trait;
use vauchi_e2e_tests::orchestrator::Orchestrator;

pub(super) struct CliDriver {
    orchestrator: Orchestrator,
}

impl CliDriver {
    pub(super) async fn start() -> Result<Self> {
        let mut orchestrator = Orchestrator::new();
        orchestrator.start().await?;
        Ok(Self { orchestrator })
    }
}

#[async_trait]
impl FeatureDriver for CliDriver {
    async fn add_user(&mut self, name: &str) -> Result<()> {
        let user = self.orchestrator.add_user(name, 1)?;
        user.read().await.create_identity().await?;
        Ok(())
    }

    async fn exchange_link(&mut self, initiator: &str, responder: &str) -> Result<()> {
        self.orchestrator
            .exchange_link(initiator, responder)
            .await?;
        Ok(())
    }

    async fn contact_names(&mut self, owner: &str) -> Result<Vec<String>> {
        let user = self
            .orchestrator
            .user(owner)
            .context("unknown scenario actor")?;
        let contacts = user.read().await.list_contacts().await?;
        Ok(contacts.into_iter().map(|contact| contact.name).collect())
    }

    async fn stop(&mut self) -> Result<()> {
        self.orchestrator.stop().await?;
        Ok(())
    }
}
