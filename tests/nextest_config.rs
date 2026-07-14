// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use serde::Deserialize;

#[derive(Deserialize)]
struct NextestConfig {
    profile: Profiles,
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

// @internal
#[test]
fn nextest_globally_isolates_resource_heavy_scenarios() {
    let config: NextestConfig = toml::from_str(include_str!("../.config/nextest.toml"))
        .expect("nextest configuration should be valid TOML");

    assert_heavy_scenarios_are_globally_isolated(&config.profile.default);
    assert_heavy_scenarios_are_globally_isolated(&config.profile.ci);
}
