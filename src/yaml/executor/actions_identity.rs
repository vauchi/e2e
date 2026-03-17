// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Identity action handlers.

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_create_identity(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let name = self.get_param_string(&step.params, "name")?;
        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;

            let device_idx = device_idx.unwrap_or(0);
            let device = user.device(device_idx).ok_or_else(|| {
                E2eError::InvalidActor(format!("Device {} not found for {}", device_idx, user_name))
            })?;
            device.read().await.create_identity(&name).await?;
        }
        Ok(None)
    }
}
