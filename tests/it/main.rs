// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Consolidated integration test binary for vauchi-e2e.

#[cfg(feature = "flame")]
#[ctor::ctor]
fn flame_init() {
    vauchi_e2e_tests::flame::init_layer();
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
