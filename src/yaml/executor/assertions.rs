// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Assertion handlers.

use std::time::Duration;

use tokio::time::timeout;

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActorRef, Assertion, AssertionStep, StepResult};

impl ScenarioExecutor {
    /// Execute an assertion step.
    pub(super) async fn execute_assertion(&self, step: &AssertionStep) -> E2eResult<StepResult> {
        let start = std::time::Instant::now();
        let description = format!("{:?}", step.assertion);

        if self.verbose {
            println!("  Assert: {}", description);
        }

        let step_timeout = step.timeout.unwrap_or(Duration::from_secs(10));

        match timeout(step_timeout, self.do_assertion(step)).await {
            Ok(Ok(())) => Ok(StepResult {
                description,
                passed: true,
                duration: start.elapsed(),
                error: None,
                output: None,
            }),
            Ok(Err(e)) => Ok(StepResult {
                description,
                passed: false,
                duration: start.elapsed(),
                error: Some(e.to_string()),
                output: None,
            }),
            Err(_) => Ok(StepResult {
                description,
                passed: false,
                duration: start.elapsed(),
                error: Some(format!("Assertion timed out after {:?}", step_timeout)),
                output: None,
            }),
        }
    }

    /// Actually perform an assertion.
    async fn do_assertion(&self, step: &AssertionStep) -> E2eResult<()> {
        let actors = step.actor.as_ref().map(|a| a.actors()).unwrap_or_default();

        match &step.assertion {
            Assertion::ContactCount | Assertion::HasContact => {
                let expected = self.get_param_usize(&step.params, "count")?;

                for actor in actors {
                    let (user_name, device_idx) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    let contacts = if let Some(idx) = device_idx {
                        user.list_contacts_on_device(idx).await?
                    } else {
                        user.list_contacts().await?
                    };

                    if contacts.len() != expected {
                        return Err(E2eError::AssertionFailed(format!(
                            "{} has {} contacts, expected {}",
                            actor,
                            contacts.len(),
                            expected
                        )));
                    }
                }
                Ok(())
            }

            Assertion::HasIdentity => {
                for actor in actors {
                    let (user_name, device_idx) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    let has_identity = if let Some(idx) = device_idx {
                        let device = user.device(idx).ok_or_else(|| {
                            E2eError::InvalidActor(format!("Device {} not found", idx))
                        })?;
                        device.read().await.has_identity().await
                    } else {
                        user.device(0)
                            .map(|d| async { d.read().await.has_identity().await })
                            .ok_or_else(|| E2eError::InvalidActor("No devices".to_string()))?
                            .await
                    };

                    if !has_identity {
                        return Err(E2eError::AssertionFailed(format!(
                            "{} does not have an identity",
                            actor
                        )));
                    }
                }
                Ok(())
            }

            Assertion::CardFieldEquals => {
                let field = self.get_param_string(&step.params, "field")?;
                let expected = self.get_param_string(&step.params, "value")?;
                let expected = self.interpolate_var(&expected);

                for actor in actors {
                    let (user_name, _) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    let card = user.get_card().await?;
                    let actual = card
                        .fields
                        .iter()
                        .find(|f| f.label == field || f.field_type == field)
                        .map(|f| &f.value);

                    if actual != Some(&expected) {
                        return Err(E2eError::AssertionFailed(format!(
                            "{} card field '{}' is {:?}, expected '{}'",
                            actor, field, actual, expected
                        )));
                    }
                }
                Ok(())
            }

            Assertion::CardsConverged => {
                // Check that all devices of each actor have the same card
                for actor in actors {
                    let (user_name, _) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    let mut reference_card: Option<crate::device::ContactCard> = None;
                    for i in 0..user.device_count() {
                        let card = user.get_card_on_device(i).await?;
                        if let Some(ref_card) = &reference_card {
                            if card.name != ref_card.name
                                || card.fields.len() != ref_card.fields.len()
                            {
                                return Err(E2eError::AssertionFailed(format!(
                                    "{}'s devices have not converged",
                                    actor
                                )));
                            }
                        } else {
                            reference_card = Some(card);
                        }
                    }
                }
                Ok(())
            }

            Assertion::DeviceCount => {
                let expected = self.get_param_usize(&step.params, "count")?;

                for actor in actors {
                    let (user_name, device_idx) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    let idx = device_idx.unwrap_or(0);
                    let device = user.device(idx).ok_or_else(|| {
                        E2eError::InvalidActor(format!("Device {} not found", idx))
                    })?;
                    let devices = device.read().await.list_devices().await?;

                    if devices.len() != expected {
                        return Err(E2eError::AssertionFailed(format!(
                            "{} has {} devices, expected {}",
                            actor,
                            devices.len(),
                            expected
                        )));
                    }
                }
                Ok(())
            }

            Assertion::CardHasField => {
                let field = self.get_param_string(&step.params, "field")?;

                for actor in actors {
                    let (user_name, _) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    let card = user.get_card().await?;
                    let has_field = card
                        .fields
                        .iter()
                        .any(|f| f.label == field || f.field_type == field);

                    if !has_field {
                        return Err(E2eError::AssertionFailed(format!(
                            "{}'s card does not have field '{}'",
                            actor, field
                        )));
                    }
                }
                Ok(())
            }

            Assertion::LabelCount => {
                let expected = self.get_param_usize(&step.params, "count")?;

                for actor in actors {
                    let (user_name, device_idx) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    let idx = device_idx.unwrap_or(0);
                    let device = user.device(idx).ok_or_else(|| {
                        E2eError::InvalidActor(format!("Device {} not found", idx))
                    })?;
                    let labels = device.read().await.list_labels().await?;

                    if labels.len() != expected {
                        return Err(E2eError::AssertionFailed(format!(
                            "{} has {} labels, expected {}",
                            actor,
                            labels.len(),
                            expected
                        )));
                    }
                }
                Ok(())
            }

            _ => Err(E2eError::InvalidStep(format!(
                "Assertion {:?} not yet implemented",
                step.assertion
            ))),
        }
    }
}
