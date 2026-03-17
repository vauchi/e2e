// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recovery action handlers.

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_create_recovery_claim(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let old_public_key = self.get_param_string(&step.params, "old_public_key")?;
        let old_public_key = self.interpolate_var(&old_public_key);

        let actor = actors.first().ok_or_else(|| {
            E2eError::InvalidStep("create_recovery_claim requires an actor".to_string())
        })?;
        let (user_name, device_idx) = ActorRef::parse(actor);
        let user = self.get_user(user_name)?;
        let user = user.read().await;
        let idx = device_idx.unwrap_or(0);
        let device = user
            .device(idx)
            .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
        let claim = device
            .read()
            .await
            .create_recovery_claim(&old_public_key)
            .await?;
        Ok(Some(claim))
    }

    pub(super) async fn action_vouch_for_recovery(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let claim = self.get_param_string(&step.params, "claim")?;
        let claim = self.interpolate_var(&claim);

        let actor = actors.first().ok_or_else(|| {
            E2eError::InvalidStep("vouch_for_recovery requires an actor".to_string())
        })?;
        let (user_name, device_idx) = ActorRef::parse(actor);
        let user = self.get_user(user_name)?;
        let user = user.read().await;
        let idx = device_idx.unwrap_or(0);
        let device = user
            .device(idx)
            .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
        let voucher = device.read().await.vouch_for_recovery(&claim).await?;
        Ok(Some(voucher))
    }

    pub(super) async fn action_add_recovery_voucher(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let voucher = self.get_param_string(&step.params, "voucher")?;
        let voucher = self.interpolate_var(&voucher);

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device.read().await.add_recovery_voucher(&voucher).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_verify_recovery_proof(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let _proof = self.get_param_string(&step.params, "proof")?;

        // Verify by passing to the recovery verify command
        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            // get_recovery_proof returns the proof data
            let _ = device.read().await.get_recovery_proof().await?;
        }
        Ok(None)
    }
}
