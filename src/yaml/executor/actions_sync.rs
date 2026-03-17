// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sync and device-linking action handlers.

use super::ScenarioExecutor;
use crate::error::E2eResult;
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_sync(&mut self, step: &ActionStep) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;

            if let Some(idx) = device_idx {
                user.sync_device(idx).await?;
            } else {
                user.sync_all().await?;
            }
        }
        Ok(None)
    }

    pub(super) async fn action_sync_device(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let device_idx = self.get_param_usize(&step.params, "device")?;
        for actor in actors {
            let (user_name, _) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            user.sync_device(device_idx).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_link_devices(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        for actor in actors {
            let (user_name, _) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            user.link_devices().await?;
        }
        Ok(None)
    }
}
