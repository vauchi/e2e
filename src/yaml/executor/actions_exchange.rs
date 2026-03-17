// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Exchange action handlers.

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_generate_qr(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let actor = actors
            .first()
            .ok_or_else(|| E2eError::InvalidStep("generate_qr requires an actor".to_string()))?;
        let (user_name, device_idx) = ActorRef::parse(actor);
        let user = self.get_user(user_name)?;
        let user = user.read().await;

        let qr = if let Some(idx) = device_idx {
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device.read().await.generate_qr().await?
        } else {
            user.generate_qr().await?
        };

        Ok(Some(qr))
    }

    pub(super) async fn action_complete_exchange(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let qr_param = self.get_param_string(&step.params, "qr")?;
        let qr = self.interpolate_var(&qr_param);

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;

            if let Some(idx) = device_idx {
                let device = user
                    .device(idx)
                    .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
                device.read().await.complete_exchange(&qr).await?;
            } else {
                user.complete_exchange(&qr).await?;
            }
        }
        Ok(None)
    }

    pub(super) async fn action_exchange(&mut self, step: &ActionStep) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        if actors.len() != 2 {
            return Err(E2eError::InvalidStep(
                "exchange requires exactly 2 actors".to_string(),
            ));
        }
        let (user_a_name, _) = ActorRef::parse(actors[0]);
        let (user_b_name, _) = ActorRef::parse(actors[1]);

        let user_a = self.get_user(user_a_name)?;
        let user_b = self.get_user(user_b_name)?;

        let user_a = user_a.read().await;
        let user_b = user_b.read().await;

        user_a.exchange_with(&user_b).await?;
        Ok(None)
    }

    pub(super) async fn action_mutual_exchange(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        if actors.len() != 2 {
            return Err(E2eError::InvalidStep(
                "mutual_exchange requires exactly 2 actors".to_string(),
            ));
        }
        let (user_a_name, _) = ActorRef::parse(actors[0]);
        let (user_b_name, _) = ActorRef::parse(actors[1]);

        let user_a = self.get_user(user_a_name)?;
        let user_b = self.get_user(user_b_name)?;

        let user_a = user_a.read().await;
        let user_b = user_b.read().await;

        user_a.mutual_exchange_with(&user_b).await?;
        Ok(None)
    }
}
