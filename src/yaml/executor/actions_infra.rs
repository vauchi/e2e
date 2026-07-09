// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Infrastructure action handlers (network, relay, wait, log).

use std::time::Duration;

use super::ScenarioExecutor;
use crate::error::{E2eError, E2eResult};
use crate::yaml::schema::{ActionStep, NetworkCondition, NetworkMode};

impl ScenarioExecutor {
    pub(super) async fn action_set_network(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let actors = step.actor_refs()?;
        let internet = self
            .get_param_bool(&step.params, "internet")
            .unwrap_or(true);
        let mode_str = self.get_param_string(&step.params, "mode").ok();
        let mode = match mode_str.as_deref() {
            Some("flaky") => NetworkMode::Flaky,
            Some("offline") => NetworkMode::Offline,
            _ => NetworkMode::Normal,
        };
        let drop_rate = self.get_param_f32(&step.params, "drop_rate").unwrap_or(0.0);

        for actor in actors {
            self.network_conditions.insert(
                actor.to_string(),
                NetworkCondition {
                    internet,
                    mode,
                    drop_rate,
                    latency_ms: 0,
                },
            );
        }
        // Note: Actual network simulation would be implemented in device
        Ok(None)
    }

    pub(super) async fn action_stop_relay(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let idx = self.get_param_usize(&step.params, "index").unwrap_or(0);
        if let Some(manager) = &mut self.relay_manager {
            manager.stop_relay(idx).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_restart_relay(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        let idx = self.get_param_usize(&step.params, "index").unwrap_or(0);
        if let Some(manager) = &mut self.relay_manager {
            manager.restart_relay(idx).await?;
        }
        Ok(None)
    }

    pub(super) async fn action_wait(&mut self, step: &ActionStep) -> E2eResult<Option<String>> {
        let duration_str = self.get_param_string(&step.params, "duration")?;
        let duration: Duration = duration_str
            .parse::<humantime::Duration>()
            .map_err(|e| E2eError::InvalidStep(format!("Invalid duration: {}", e)))?
            .into();
        tokio::time::sleep(duration).await;
        Ok(None)
    }

    pub(super) async fn action_log(&mut self, step: &ActionStep) -> E2eResult<Option<String>> {
        let message = self.get_param_string(&step.params, "message")?;
        let interpolated = self.interpolate_var(&message);
        println!("LOG: {}", interpolated);
        Ok(None)
    }

    /// Stub for network-partition actions used by manual-only edge scenarios.
    ///
    /// These actions require OS-level traffic control between relay
    /// processes, which the current test harness does not implement. They
    /// are kept parseable so the YAML loader can read the scenarios, and
    /// skipped at execution time when the scenario is marked `manual_only`.
    /// If a non-manual scenario tries to use them, we fail loudly instead of
    /// silently returning success.
    pub(super) async fn action_network_partition_stub(
        &mut self,
        step: &ActionStep,
    ) -> E2eResult<Option<String>> {
        Err(E2eError::InvalidStep(format!(
            "Network-partition action {:?} is not yet implemented in the automated harness",
            step.action
        )))
    }
}
