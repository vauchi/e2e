// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::FeatureDriver;
use anyhow::{Result, bail};
use async_trait::async_trait;

pub(super) struct ContractDriver;

#[async_trait]
impl FeatureDriver for ContractDriver {
    async fn add_user(&mut self, _name: &str) -> Result<()> {
        bail!("contract scenario driver has not been implemented")
    }
    async fn exchange_link(&mut self, _initiator: &str, _responder: &str) -> Result<()> {
        bail!("contract scenario driver has not been implemented")
    }
    async fn contact_names(&mut self, _owner: &str) -> Result<Vec<String>> {
        bail!("contract scenario driver has not been implemented")
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}
