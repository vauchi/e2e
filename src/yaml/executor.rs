//! YAML Scenario Executor
//!
//! Executes YAML scenario files by interpreting steps and coordinating devices.

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
    Action, ActorRef, Assertion, AssertionStep, ActionStep, ConditionalStep,
    NetworkCondition, NetworkMode, ParallelStep, Platform, Scenario,
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
        let result = timeout(scenario.timeout, self.execute_steps(&scenario.steps))
            .await;

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
            self.network_conditions.insert(actor.clone(), condition.clone());
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = E2eResult<StepResult>> + Send + 'a>> {
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
                        output: self.outputs.get(step.output.as_ref().unwrap_or(&String::new())).cloned(),
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

    /// Actually perform an action.
    async fn do_action(&mut self, step: &ActionStep) -> E2eResult<Option<String>> {
        let actors = step.actor.actors();

        match &step.action {
            Action::CreateIdentity => {
                let name = self.get_param_string(&step.params, "name")?;
                for actor in actors {
                    let (user_name, device_idx) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    // Get the device (primary or specific)
                    let device_idx = device_idx.unwrap_or(0);
                    let device = user.device(device_idx).ok_or_else(|| {
                        E2eError::InvalidActor(format!("Device {} not found for {}", device_idx, user_name))
                    })?;
                    device.read().await.create_identity(&name).await?;
                }
                Ok(None)
            }

            Action::GenerateQr => {
                let actor = actors.first().ok_or_else(|| {
                    E2eError::InvalidStep("generate_qr requires an actor".to_string())
                })?;
                let (user_name, device_idx) = ActorRef::parse(actor);
                let user = self.get_user(user_name)?;
                let user = user.read().await;

                let qr = if let Some(idx) = device_idx {
                    let device = user.device(idx).ok_or_else(|| {
                        E2eError::InvalidActor(format!("Device {} not found", idx))
                    })?;
                    device.read().await.generate_qr().await?
                } else {
                    user.generate_qr().await?
                };

                Ok(Some(qr))
            }

            Action::CompleteExchange => {
                let qr_param = self.get_param_string(&step.params, "qr")?;
                let qr = self.interpolate_var(&qr_param);

                for actor in actors {
                    let (user_name, device_idx) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;

                    if let Some(idx) = device_idx {
                        let device = user.device(idx).ok_or_else(|| {
                            E2eError::InvalidActor(format!("Device {} not found", idx))
                        })?;
                        device.read().await.complete_exchange(&qr).await?;
                    } else {
                        user.complete_exchange(&qr).await?;
                    }
                }
                Ok(None)
            }

            Action::Sync | Action::SyncAll => {
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

            Action::SyncDevice => {
                let device_idx = self.get_param_usize(&step.params, "device")?;
                for actor in actors {
                    let (user_name, _) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;
                    user.sync_device(device_idx).await?;
                }
                Ok(None)
            }

            Action::AddField => {
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

            Action::UpdateCard => {
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

            Action::SetNetwork => {
                let internet = self.get_param_bool(&step.params, "internet").unwrap_or(true);
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

            Action::StopRelay => {
                let idx = self.get_param_usize(&step.params, "index").unwrap_or(0);
                if let Some(manager) = &mut self.relay_manager {
                    manager.stop_relay(idx).await?;
                }
                Ok(None)
            }

            Action::RestartRelay => {
                let idx = self.get_param_usize(&step.params, "index").unwrap_or(0);
                if let Some(manager) = &mut self.relay_manager {
                    manager.restart_relay(idx).await?;
                }
                Ok(None)
            }

            Action::Wait => {
                let duration_str = self.get_param_string(&step.params, "duration")?;
                let duration: Duration = duration_str
                    .parse::<humantime::Duration>()
                    .map_err(|e| E2eError::InvalidStep(format!("Invalid duration: {}", e)))?
                    .into();
                tokio::time::sleep(duration).await;
                Ok(None)
            }

            Action::Log => {
                let message = self.get_param_string(&step.params, "message")?;
                let interpolated = self.interpolate_var(&message);
                println!("LOG: {}", interpolated);
                Ok(None)
            }

            Action::LinkDevices => {
                for actor in actors {
                    let (user_name, _) = ActorRef::parse(actor);
                    let user = self.get_user(user_name)?;
                    let user = user.read().await;
                    user.link_devices().await?;
                }
                Ok(None)
            }

            Action::Exchange => {
                // Exchange between two users
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

            Action::MutualExchange => {
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

            _ => {
                // Actions not yet implemented
                Err(E2eError::InvalidStep(format!(
                    "Action {:?} not yet implemented",
                    step.action
                )))
            }
        }
    }

    /// Execute an assertion step.
    async fn execute_assertion(&self, step: &AssertionStep) -> E2eResult<StepResult> {
        let start = Instant::now();
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
        let actors = step
            .actor
            .as_ref()
            .map(|a| a.actors())
            .unwrap_or_default();

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
                            if card.name != ref_card.name || card.fields.len() != ref_card.fields.len() {
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

            _ => Err(E2eError::InvalidStep(format!(
                "Assertion {:?} not yet implemented",
                step.assertion
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
        self.users.get(name).cloned().ok_or_else(|| {
            E2eError::InvalidActor(format!("User '{}' not found", name))
        })
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

    fn get_param_f32(
        &self,
        params: &HashMap<String, serde_yaml::Value>,
        key: &str,
    ) -> Option<f32> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_var() {
        let mut executor = ScenarioExecutor::new();
        executor.outputs.insert("qr_data".to_string(), "ABC123".to_string());

        assert_eq!(executor.interpolate_var("${qr_data}"), "ABC123");
        assert_eq!(executor.interpolate_var("QR: ${qr_data}"), "QR: ABC123");
        assert_eq!(executor.interpolate_var("no vars"), "no vars");
    }

    #[test]
    fn test_evaluate_condition() {
        let mut executor = ScenarioExecutor::new();
        executor.outputs.insert("flag".to_string(), "true".to_string());

        assert!(executor.evaluate_condition("${flag}"));
        assert!(!executor.evaluate_condition("${undefined}"));
    }
}
