// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! YAML Scenario Runner
//!
//! Loads and executes YAML scenario files from `e2e/scenarios/`.
//! Each scenario tier (smoke, integration, edge) runs as a separate test.
//!
//! ## Usage
//!
//! Run all YAML scenarios:
//! ```bash
//! cargo test -p vauchi-e2e-tests --test yaml_scenarios -- --ignored
//! ```
//!
//! Run only smoke scenarios:
//! ```bash
//! cargo test -p vauchi-e2e-tests --test yaml_scenarios smoke -- --ignored
//! ```

use vauchi_e2e_tests::prelude::*;

/// Run all smoke-tier YAML scenarios.
#[tokio::test]
async fn yaml_smoke_scenarios() {
    let mut loader = ScenarioLoader::new_default();
    let scenarios = loader
        .load_dir("smoke")
        .expect("Failed to load smoke scenarios");

    assert!(!scenarios.is_empty(), "No smoke scenarios found");

    let mut executor = ScenarioExecutor::new();
    for scenario in &scenarios {
        if scenario.manual_only {
            ScenarioExecutor::print_manual_instructions(scenario);
            continue;
        }
        let result = executor.execute(scenario).await.expect("Executor failed");
        assert!(
            result.passed,
            "Smoke scenario '{}' failed: {}",
            result.name,
            result.error.unwrap_or_default()
        );
    }
}

/// Run all integration-tier YAML scenarios.
#[tokio::test]
async fn yaml_integration_scenarios() {
    let mut loader = ScenarioLoader::new_default();
    let scenarios = loader
        .load_dir("integration")
        .expect("Failed to load integration scenarios");

    assert!(!scenarios.is_empty(), "No integration scenarios found");

    let mut executor = ScenarioExecutor::new();
    for scenario in &scenarios {
        if scenario.manual_only {
            ScenarioExecutor::print_manual_instructions(scenario);
            continue;
        }
        let result = executor.execute(scenario).await.expect("Executor failed");
        assert!(
            result.passed,
            "Integration scenario '{}' failed: {}",
            result.name,
            result.error.unwrap_or_default()
        );
    }
}

/// Run all edge-case YAML scenarios.
#[tokio::test]
#[ignore = "edge scenarios use unimplemented YAML actions (partition_network, offline_exchange)"]
async fn yaml_edge_scenarios() {
    let mut loader = ScenarioLoader::new_default();
    let scenarios = loader
        .load_dir("edge")
        .expect("Failed to load edge scenarios");

    assert!(!scenarios.is_empty(), "No edge scenarios found");

    let mut executor = ScenarioExecutor::new();
    for scenario in &scenarios {
        if scenario.manual_only {
            ScenarioExecutor::print_manual_instructions(scenario);
            continue;
        }
        let result = executor.execute(scenario).await.expect("Executor failed");
        assert!(
            result.passed,
            "Edge scenario '{}' failed: {}",
            result.name,
            result.error.unwrap_or_default()
        );
    }
}

/// Verify all YAML scenarios can be loaded and parsed (no runtime infra needed).
#[tokio::test]
async fn yaml_scenarios_parse_successfully() {
    let mut loader = ScenarioLoader::new_default();
    let scenarios = loader.load_all().expect("Failed to load all scenarios");

    assert!(
        !scenarios.is_empty(),
        "No YAML scenarios found in scenarios/ directory"
    );

    // Verify each scenario has required fields
    for scenario in &scenarios {
        assert!(!scenario.name.is_empty(), "Scenario must have a name");
        assert!(
            !scenario.steps.is_empty(),
            "Scenario '{}' must have steps",
            scenario.name
        );
    }
}
