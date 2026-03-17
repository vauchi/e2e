// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Contact action handlers.

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_list_contacts(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let actor = actors
            .first()
            .ok_or_else(|| E2eError::InvalidStep("list_contacts requires an actor".to_string()))?;
        let (user_name, _) = ActorRef::parse(actor);
        let user = self.get_user(user_name)?;
        let user = user.read().await;
        let contacts = user.list_contacts().await?;
        Ok(Some(format!("{}", contacts.len())))
    }

    pub(super) async fn action_get_contact(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let name = self.get_param_string(&step.params, "name")?;
        let actor = actors
            .first()
            .ok_or_else(|| E2eError::InvalidStep("get_contact requires an actor".to_string()))?;
        let (user_name, _) = ActorRef::parse(actor);
        let user = self.get_user(user_name)?;
        let user = user.read().await;
        let _contact = user.get_contact(&name).await?;
        Ok(None)
    }

    pub(super) async fn action_verify_contact(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let contact = self.get_param_string(&step.params, "contact")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device.read().await.verify_contact(&contact).await?;
        }
        Ok(None)
    }
}
