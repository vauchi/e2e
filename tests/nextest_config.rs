// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct NextestConfig {
    #[serde(rename = "test-groups")]
    test_groups: HashMap<String, TestGroup>,
    profile: Profiles,
}

#[derive(Deserialize)]
struct TestGroup {
    #[serde(rename = "max-threads")]
    max_threads: usize,
}

#[derive(Deserialize)]
struct Profiles {
    default: Profile,
    ci: Profile,
}

#[derive(Deserialize)]
struct Profile {
    overrides: Vec<Override>,
}

#[derive(Deserialize)]
struct Override {
    filter: String,
    #[serde(rename = "threads-required")]
    threads_required: Option<String>,
    #[serde(rename = "test-group")]
    test_group: Option<String>,
}

fn assert_heavy_scenarios_are_globally_isolated(profile: &Profile) {
    let isolation = profile.overrides.iter().find(|candidate| {
        candidate
            .filter
            .contains("five_user_exchange::integration_five_user_exchange")
            && candidate
                .filter
                .contains("onboarding_flow::test_time_to_value_under_2_minutes")
    });

    assert_eq!(
        isolation.and_then(|candidate| candidate.threads_required.as_deref()),
        Some("num-test-threads")
    );
}

fn assert_integration_tests_are_rate_limited(profile: &Profile) {
    let rate_limit = profile
        .overrides
        .iter()
        .find(|candidate| candidate.filter == "binary(=it)");

    assert_eq!(
        rate_limit.and_then(|candidate| candidate.test_group.as_deref()),
        Some("relay-tests")
    );
}

// @internal
#[test]
fn nextest_globally_isolates_resource_heavy_scenarios() {
    let config: NextestConfig = toml::from_str(include_str!("../.config/nextest.toml"))
        .expect("nextest configuration should be valid TOML");

    assert_heavy_scenarios_are_globally_isolated(&config.profile.default);
    assert_heavy_scenarios_are_globally_isolated(&config.profile.ci);
    assert_integration_tests_are_rate_limited(&config.profile.default);
    assert_integration_tests_are_rate_limited(&config.profile.ci);
    assert_eq!(config.test_groups["relay-tests"].max_threads, 4);
}
