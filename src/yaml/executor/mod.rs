// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! YAML Scenario Executor
//!
//! Executes YAML scenario files by interpreting steps and coordinating devices.

mod actions_backup;
mod actions_card;
mod actions_contacts;
mod actions_exchange;
mod actions_identity;
mod actions_infra;
mod actions_labels;
mod actions_recovery;
mod actions_sync;
mod assertions;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tokio::time::timeout;

use crate::device::DeviceType;
use crate::error::{E2eError, E2eResult};
use crate::relay_manager::{RelayConfig as RMConfig, RelayManager};
use crate::user::User;

use super::schema::{
    Action, ActionStep, ConditionalStep, NetworkCondition, ParallelStep, Platform, Scenario,
    ScenarioResult, Step, StepResult, WaitStep,
};

/// Executor for YAML scenarios.
pub struct ScenarioExecutor {
    /// Relay manager.
    relay_manager: Option<RelayManager>,

    /// Users in this scenario.
    users: HashMap<String, Arc<RwLock<User>>>,

    /// Output variables collected during execution.
    outputs: HashMap<String, String>,

    /// Current network conditions per actor.
    network_conditions: HashMap<String, NetworkCondition>,

    /// Whether to print verbose output.
    verbose: bool,
}

impl ScenarioExecutor {
    /// Create a new executor.
    pub fn new() -> Self {
        Self {
            relay_manager: None,
            users: HashMap::new(),
            outputs: HashMap::new(),
            network_conditions: HashMap::new(),
            verbose: std::env::var("E2E_VERBOSE").is_ok(),
        }
    }

    /// Execute a scenario.
    pub async fn execute(&mut self, scenario: &Scenario) -> E2eResult<ScenarioResult> {
        let start = Instant::now();
        let mut step_results = Vec::new();

        if self.verbose {
            println!("Executing scenario: {}", scenario.name);
        }

        // Setup phase
        if let Err(e) = self.setup(scenario).await {
            return Ok(ScenarioResult {
                name: scenario.name.clone(),
                passed: false,
                duration: start.elapsed(),
                steps: step_results,
                error: Some(format!("Setup failed: {}", e)),
                outputs: self.outputs.clone(),
            });
        }

        // Execute steps with scenario timeout
        let result = timeout(scenario.timeout, self.execute_steps(&scenario.steps)).await;

        let (passed, error, steps) = match result {
            Ok(Ok(steps)) => (true, None, steps),
            Ok(Err(e)) => (false, Some(e.to_string()), vec![]),
            Err(_) => (
                false,
                Some(format!("Scenario timed out after {:?}", scenario.timeout)),
                vec![],
            ),
        };

        step_results.extend(steps);

        // Cleanup
        if let Err(e) = self.cleanup().await {
            if self.verbose {
                eprintln!("Cleanup error: {}", e);
            }
        }

        Ok(ScenarioResult {
            name: scenario.name.clone(),
            passed,
            duration: start.elapsed(),
            steps: step_results,
            error,
            outputs: self.outputs.clone(),
        })
    }

    /// Print manual instructions for a scenario.
    pub fn print_manual_instructions(scenario: &Scenario) {
        println!("Manual Test: {}", scenario.name);
        println!("{}", "=".repeat(50));

        if let Some(desc) = &scenario.description {
            println!("\n{}\n", desc);
        }

        println!("Participants:");
        for (name, participant) in &scenario.participants {
            println!(
                "  - {} ({} device(s), {})",
                name, participant.devices, participant.platform
            );
        }

        println!("\nSteps:");
        for (i, instruction) in scenario.manual_instructions.iter().enumerate() {
            println!("  {}. {}", i + 1, instruction);
        }

        if let Some(feature) = &scenario.feature {
            println!("\nGherkin Reference: {}", feature);
        }

        println!("\n{}", "=".repeat(50));
    }

    /// Setup the scenario infrastructure.
    async fn setup(&mut self, scenario: &Scenario) -> E2eResult<()> {
        // Setup relays
        let relay_config = scenario.relay_config.as_ref();
        let relay_count = relay_config.map(|c| c.count).unwrap_or(1);

        if relay_count > 0 {
            let mut config = RMConfig::default();
            if let Some(rc) = relay_config {
                if let Some(port) = rc.base_port {
                    config.base_port = port;
                }
                if let Some(backend) = &rc.storage_backend {
                    config.storage_backend = backend.clone();
                }
            }

            let mut manager = RelayManager::with_config(config).await?;
            manager.spawn(relay_count).await?;
            self.relay_manager = Some(manager);
        }

        // Setup users
        let relay_url = self
            .relay_manager
            .as_ref()
            .and_then(|m| m.relay_url(0))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "ws://localhost:8080".to_string());

        for (name, participant) in &scenario.participants {
            let mut user = User::new(name);

            // Add devices based on platform
            let device_type = match participant.platform {
                Platform::Any | Platform::Cli => DeviceType::Cli,
                Platform::Ios => DeviceType::IosSimulator,
                Platform::Android => DeviceType::AndroidEmulator,
                Platform::Desktop => DeviceType::Desktop,
                Platform::Tui => DeviceType::Tui,
            };

            for _ in 0..participant.devices {
                match device_type {
                    DeviceType::Cli => {
                        user.add_cli_device(&relay_url)?;
                    }
                    _ => {
                        // TODO: Add other device types when implemented
                        return Err(E2eError::DeviceNotSupported(format!(
                            "Device type {:?} not yet implemented",
                            device_type
                        )));
                    }
                }
            }

            self.users.insert(name.clone(), Arc::new(RwLock::new(user)));
        }

        // Apply initial network conditions
        for (actor, condition) in &scenario.network_conditions {
            self.network_conditions
                .insert(actor.clone(), condition.clone());
        }

        Ok(())
    }

    /// Execute a list of steps.
    async fn execute_steps(&mut self, steps: &[Step]) -> E2eResult<Vec<StepResult>> {
        let mut results = Vec::new();

        for step in steps {
            let result = self.execute_step(step).await?;
            let passed = result.passed;
            results.push(result);

            if !passed {
                break; // Stop on first failure
            }
        }

        Ok(results)
    }

    /// Execute a single step.
    fn execute_step<'a>(
        &'a mut self,
        step: &'a Step,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = E2eResult<StepResult>> + Send + 'a>>
    {
        Box::pin(async move {
            match step {
                Step::Action(action_step) => self.execute_action(action_step).await,
                Step::Assertion(assertion_step) => self.execute_assertion(assertion_step).await,
                Step::Wait(wait_step) => self.execute_wait(wait_step).await,
                Step::Parallel(parallel_step) => self.execute_parallel(parallel_step).await,
                Step::If(conditional_step) => self.execute_conditional(conditional_step).await,
            }
        })
    }

    /// Execute an action step.
    async fn execute_action(&mut self, step: &ActionStep) -> E2eResult<StepResult> {
        let start = Instant::now();
        let description = format!("{:?} on {:?}", step.action, step.actor);

        if self.verbose {
            println!("  Action: {}", description);
        }

        // Get timeout for this step
        let step_timeout = step.timeout.unwrap_or(Duration::from_secs(30));

        // Execute with retries
        let mut last_error = None;
        for attempt in 0..=step.retries {
            if attempt > 0 && self.verbose {
                println!("    Retry {} of {}", attempt, step.retries);
            }

            match timeout(step_timeout, self.do_action(step)).await {
                Ok(Ok(output)) => {
                    // Store output if specified
                    if let Some(output_var) = &step.output {
                        if let Some(value) = output {
                            self.outputs.insert(output_var.clone(), value.clone());
                        }
                    }

                    return Ok(StepResult {
                        description,
                        passed: step.expect_error.is_none(),
                        duration: start.elapsed(),
                        error: None,
                        output: self
                            .outputs
                            .get(step.output.as_ref().unwrap_or(&String::new()))
                            .cloned(),
                    });
                }
                Ok(Err(e)) => {
                    // Check if this error was expected
                    if let Some(expected) = &step.expect_error {
                        if e.to_string().contains(expected) {
                            return Ok(StepResult {
                                description,
                                passed: true,
                                duration: start.elapsed(),
                                error: None,
                                output: None,
                            });
                        }
                    }
                    last_error = Some(e);
                }
                Err(_) => {
                    last_error = Some(E2eError::Timeout(format!(
                        "Action {:?} timed out after {:?}",
                        step.action, step_timeout
                    )));
                }
            }
        }

        let error = last_error.map(|e| e.to_string());
        Ok(StepResult {
            description,
            passed: false,
            duration: start.elapsed(),
            error,
            output: None,
        })
    }

    /// Dispatch an action to the appropriate handler.
    async fn do_action(&mut self, step: &ActionStep) -> E2eResult<Option<String>> {
        match &step.action {
            Action::CreateIdentity => self.action_create_identity(step).await,
            Action::GenerateQr => self.action_generate_qr(step).await,
            Action::CompleteExchange => self.action_complete_exchange(step).await,
            Action::Exchange => self.action_exchange(step).await,
            Action::MutualExchange => self.action_mutual_exchange(step).await,
            Action::Sync | Action::SyncAll => self.action_sync(step).await,
            Action::SyncDevice => self.action_sync_device(step).await,
            Action::LinkDevices => self.action_link_devices(step).await,
            Action::AddField => self.action_add_field(step).await,
            Action::UpdateCard => self.action_update_card(step).await,
            Action::GetCard => self.action_get_card(step).await,
            Action::EditField => self.action_edit_field(step).await,
            Action::RemoveField => self.action_remove_field(step).await,
            Action::EditName => self.action_edit_name(step).await,
            Action::ListContacts => self.action_list_contacts(step).await,
            Action::GetContact => self.action_get_contact(step).await,
            Action::VerifyContact => self.action_verify_contact(step).await,
            Action::CreateLabel => self.action_create_label(step).await,
            Action::DeleteLabel => self.action_delete_label(step).await,
            Action::AddContactToLabel => self.action_add_contact_to_label(step).await,
            Action::RemoveContactFromLabel => self.action_remove_contact_from_label(step).await,
            Action::ShowFieldToLabel => self.action_show_field_to_label(step).await,
            Action::HideFieldFromLabel => self.action_hide_field_from_label(step).await,
            Action::HideFieldFromContact => self.action_hide_field_from_contact(step).await,
            Action::UnhideFieldToContact => self.action_unhide_field_to_contact(step).await,
            Action::CreateRecoveryClaim => self.action_create_recovery_claim(step).await,
            Action::VouchForRecovery => self.action_vouch_for_recovery(step).await,
            Action::AddRecoveryVoucher => self.action_add_recovery_voucher(step).await,
            Action::VerifyRecoveryProof => self.action_verify_recovery_proof(step).await,
            Action::ExportBackup => self.action_export_backup(step).await,
            Action::ImportBackup => self.action_import_backup(step).await,
            Action::SetNetwork => self.action_set_network(step).await,
            Action::StopRelay => self.action_stop_relay(step).await,
            Action::RestartRelay => self.action_restart_relay(step).await,
            Action::Wait => self.action_wait(step).await,
            Action::Log => self.action_log(step).await,
            _ => Err(E2eError::InvalidStep(format!(
                "Action {:?} not yet implemented",
                step.action
            ))),
        }
    }

    /// Execute a wait step.
    async fn execute_wait(&self, step: &WaitStep) -> E2eResult<StepResult> {
        let start = Instant::now();
        let description = format!(
            "Wait {:?}{}",
            step.duration,
            step.reason
                .as_ref()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default()
        );

        if self.verbose {
            println!("  {}", description);
        }

        tokio::time::sleep(step.duration).await;

        Ok(StepResult {
            description,
            passed: true,
            duration: start.elapsed(),
            error: None,
            output: None,
        })
    }

    /// Execute steps in parallel.
    async fn execute_parallel(&mut self, step: &ParallelStep) -> E2eResult<StepResult> {
        let start = Instant::now();
        let description = format!("Parallel ({} steps)", step.steps.len());

        // Note: For true parallel execution, we'd need to restructure the executor
        // For now, execute sequentially but collect results
        let mut all_passed = true;
        let mut errors = Vec::new();

        for s in &step.steps {
            let result = self.execute_step(s).await?;
            if !result.passed {
                all_passed = false;
                if let Some(e) = result.error {
                    errors.push(e);
                }
                if step.all_must_succeed {
                    break;
                }
            }
        }

        Ok(StepResult {
            description,
            passed: all_passed || !step.all_must_succeed,
            duration: start.elapsed(),
            error: if errors.is_empty() {
                None
            } else {
                Some(errors.join("; "))
            },
            output: None,
        })
    }

    /// Execute a conditional step.
    async fn execute_conditional(&mut self, step: &ConditionalStep) -> E2eResult<StepResult> {
        let start = Instant::now();
        let description = format!("If: {}", step.condition);

        // Evaluate condition (simple variable check for now)
        let condition_met = self.evaluate_condition(&step.condition);

        let steps = if condition_met {
            &step.then
        } else {
            &step.otherwise
        };

        let mut all_passed = true;
        for s in steps {
            let result = self.execute_step(s).await?;
            if !result.passed {
                all_passed = false;
                break;
            }
        }

        Ok(StepResult {
            description,
            passed: all_passed,
            duration: start.elapsed(),
            error: None,
            output: None,
        })
    }

    /// Cleanup after scenario execution.
    async fn cleanup(&mut self) -> E2eResult<()> {
        // Stop relays
        if let Some(manager) = &mut self.relay_manager {
            manager.stop_all().await;
        }

        // Clear state
        self.users.clear();
        self.outputs.clear();
        self.network_conditions.clear();

        Ok(())
    }

    // Helper methods

    fn get_user(&self, name: &str) -> E2eResult<Arc<RwLock<User>>> {
        self.users
            .get(name)
            .cloned()
            .ok_or_else(|| E2eError::InvalidActor(format!("User '{}' not found", name)))
    }

    fn get_param_string(
        &self,
        params: &HashMap<String, serde_yaml::Value>,
        key: &str,
    ) -> E2eResult<String> {
        params
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| self.interpolate_var(s))
            .ok_or_else(|| E2eError::InvalidStep(format!("Missing parameter: {}", key)))
    }

    fn get_param_usize(
        &self,
        params: &HashMap<String, serde_yaml::Value>,
        key: &str,
    ) -> E2eResult<usize> {
        params
            .get(key)
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| E2eError::InvalidStep(format!("Missing parameter: {}", key)))
    }

    fn get_param_bool(
        &self,
        params: &HashMap<String, serde_yaml::Value>,
        key: &str,
    ) -> Option<bool> {
        params.get(key).and_then(|v| v.as_bool())
    }

    fn get_param_f32(&self, params: &HashMap<String, serde_yaml::Value>, key: &str) -> Option<f32> {
        params.get(key).and_then(|v| v.as_f64()).map(|n| n as f32)
    }

    fn interpolate_var(&self, s: &str) -> String {
        let mut result = s.to_string();
        for (key, value) in &self.outputs {
            result = result.replace(&format!("${{{}}}", key), value);
        }
        result
    }

    fn evaluate_condition(&self, condition: &str) -> bool {
        // Simple condition evaluation: check if variable exists and is non-empty
        if condition.starts_with("${") && condition.ends_with("}") {
            let var_name = &condition[2..condition.len() - 1];
            return self
                .outputs
                .get(var_name)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
        }
        false
    }
}

impl Default for ScenarioExecutor {
    fn default() -> Self {
        Self::new()
    }
}

// INLINE_TEST_REQUIRED: tests exercise private interpolation/condition helpers
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_var() {
        let mut executor = ScenarioExecutor::new();
        executor
            .outputs
            .insert("qr_data".to_string(), "ABC123".to_string());

        assert_eq!(executor.interpolate_var("${qr_data}"), "ABC123");
        assert_eq!(executor.interpolate_var("QR: ${qr_data}"), "QR: ABC123");
        assert_eq!(executor.interpolate_var("no vars"), "no vars");
    }

    #[test]
    fn test_evaluate_condition() {
        let mut executor = ScenarioExecutor::new();
        executor
            .outputs
            .insert("flag".to_string(), "true".to_string());

        assert!(executor.evaluate_condition("${flag}"));
        assert!(!executor.evaluate_condition("${undefined}"));
    }
}
