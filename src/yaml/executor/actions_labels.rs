// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Visibility label and contact visibility action handlers.

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_create_label(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let name = self.get_param_string(&step.params, "name")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device.read().await.create_label(&name).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_delete_label(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let name = self.get_param_string(&step.params, "name")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device.read().await.delete_label(&name).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_add_contact_to_label(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let label = self.get_param_string(&step.params, "label")?;
        let contact = self.get_param_string(&step.params, "contact")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device
                .read()
                .await
                .add_contact_to_label(&label, &contact)
                .await?;
        }
        Ok(None)
    }

    pub(super) async fn action_remove_contact_from_label(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let label = self.get_param_string(&step.params, "label")?;
        let contact = self.get_param_string(&step.params, "contact")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device
                .read()
                .await
                .remove_contact_from_label(&label, &contact)
                .await?;
        }
        Ok(None)
    }

    pub(super) async fn action_show_field_to_label(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let label = self.get_param_string(&step.params, "label")?;
        let field = self.get_param_string(&step.params, "field")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device
                .read()
                .await
                .show_field_to_label(&label, &field)
                .await?;
        }
        Ok(None)
    }

    pub(super) async fn action_hide_field_from_label(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let label = self.get_param_string(&step.params, "label")?;
        let field = self.get_param_string(&step.params, "field")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device
                .read()
                .await
                .hide_field_from_label(&label, &field)
                .await?;
        }
        Ok(None)
    }

    pub(super) async fn action_hide_field_from_contact(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let contact = self.get_param_string(&step.params, "contact")?;
        let field = self.get_param_string(&step.params, "field")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device
                .read()
                .await
                .hide_field_from_contact(&contact, &field)
                .await?;
        }
        Ok(None)
    }

    pub(super) async fn action_unhide_field_to_contact(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let contact = self.get_param_string(&step.params, "contact")?;
        let field = self.get_param_string(&step.params, "field")?;

        for actor in actors {
            let (user_name, device_idx) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            let idx = device_idx.unwrap_or(0);
            let device = user
                .device(idx)
                .ok_or_else(|| E2eError::InvalidActor(format!("Device {} not found", idx)))?;
            device
                .read()
                .await
                .unhide_field_to_contact(&contact, &field)
                .await?;
        }
        Ok(None)
    }
}
