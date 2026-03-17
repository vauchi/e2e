// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backup action handlers.

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_export_backup(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let password = self.get_param_string(&step.params, "password")?;

        let actor = actors
            .first()
            .ok_or_else(|| E2eError::InvalidStep("export_backup requires an actor".to_string()))?;
        let (user_name, device_idx) = ActorRef::parse(actor);
        let user = self.get_user(user_name)?;
        let user = user.read().await;
        let idx = device_idx.unwrap_or(0);
        let device = user
            .device(idx)
            .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
        let path = device.read().await.export_backup(&password).await?;
        Ok(Some(path))
    }

    pub(super) async fn action_import_backup(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let path = self.get_param_string(&step.params, "path")?;
        let path = self.interpolate_var(&path);
        let password = self.get_param_string(&step.params, "password")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device.read().await.import_backup(&path, &password).await?;
        }
        Ok(None)
    }
}
