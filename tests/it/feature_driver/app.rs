// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::FeatureDriver;
use anyhow::{Context, Result};
use async_trait::async_trait;
use vauchi_e2e_tests::orchestrator::Orchestrator;

pub(super) enum AppKind {
    Cli,
    #[cfg(feature = "tui")]
    Tui,
}

pub(super) struct AppDriver {
    orchestrator: Orchestrator,
    kind: AppKind,
}

impl AppDriver {
    pub(super) async fn start(kind: AppKind) -> Result<Self> {
        let mut orchestrator = Orchestrator::new();
        orchestrator.start().await?;
        Ok(Self { orchestrator, kind })
    }
}

#[async_trait]
impl FeatureDriver for AppDriver {
    async fn add_user(&mut self, name: &str) -> Result<()> {
        let user = match self.kind {
            AppKind::Cli => self.orchestrator.add_user(name, 1)?,
            #[cfg(feature = "tui")]
            AppKind::Tui => self.orchestrator.add_user_with_tui(name)?,
        };
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
