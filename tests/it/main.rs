// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Consolidated integration test binary for vauchi-e2e.

/// Install the test tracing subscriber once at process start.
///
/// Always installs a fmt layer so subprocess stderr forwarded by the
/// orchestrator (`tracing::warn!(target = "relay", ...)`) reaches the
/// test output. With `--features flame`, also installs a tracing-flame
/// layer for `.folded` profile capture.
#[ctor::ctor]
fn tracing_init() {
    vauchi_e2e_tests::test_logging::init();
}

mod contact_actions;
mod cross_platform;
mod delivery_pipeline;
mod exchange_error_paths;
mod five_user_exchange;
mod multi_device_sync;
mod offline_catchup;
mod ohttp_advanced;
mod ohttp_helpers;
mod ohttp_integration;
mod onboarding_flow;
mod recovery_flow;
mod relay_failover;
mod resistance_features;
mod seeded_contacts;
mod version_enforcement_tests;
mod visibility_labels;
mod yaml_scenarios;
