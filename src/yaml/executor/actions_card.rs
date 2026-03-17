// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Card field action handlers.

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, ActorRef};

impl ScenarioExecutor {
    pub(super) async fn action_add_field(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let field_type = self.get_param_string(&step.params, "field_type")?;
        let label = self.get_param_string(&step.params, "label")?;
        let value = self.get_param_string(&step.params, "value")?;

        for actor in actors {
            let (user_name, _) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            user.add_field(&field_type, &label, &value).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_update_card(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let field = self.get_param_string(&step.params, "field")?;
        let value = self.get_param_string(&step.params, "value")?;

        for actor in actors {
            let (user_name, _) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            user.edit_field(&field, &value).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_get_card(&mut self, step: &ActionStep) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let actor = actors
            .first()
            .ok_or_else(|| E2eError::InvalidStep("get_card requires an actor".to_string()))?;
        let (user_name, _) = ActorRef::parse(actor);
        let user = self.get_user(user_name)?;
        let user = user.read().await;
        let _card = user.get_card().await?;
        Ok(None)
    }

    pub(super) async fn action_edit_field(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let label = self.get_param_string(&step.params, "label")?;
        let value = self.get_param_string(&step.params, "value")?;

        for actor in actors {
            let (user_name, _) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            user.edit_field(&label, &value).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_remove_field(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let label = self.get_param_string(&step.params, "label")?;

        for actor in actors {
            let (user_name, _) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            user.remove_field(&label).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_edit_name(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();
        let name = self.get_param_string(&step.params, "name")?;

        for actor in actors {
            let (user_name, _) = ActorRef::parse(actor);
            let user = self.get_user(user_name)?;
            let user = user.read().await;
            user.edit_name(&name).await?;
        }
        Ok(None)
    }
}
