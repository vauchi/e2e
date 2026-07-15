// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

const CI_CONFIG: &str = include_str!("../.gitlab-ci.yml");
const RG3_TEST: &str =
    "orchestrator_default_ohttp::integration_ohttp_split_relay_config_routes_via_ohttp_relay";
const RG4_RG5_TEST: &str =
    "multi_device_sync::integration_six_device_exchange_and_update_convergence";

fn top_level_job(name: &str) -> &str {
    let marker = format!("{name}:\n");
    let start = CI_CONFIG
        .find(&marker)
        .unwrap_or_else(|| panic!("missing {name} job"));
    let remainder = &CI_CONFIG[start + marker.len()..];
    let mut end = remainder.len();
    let mut offset = 0;
    for line in remainder.split_inclusive('\n') {
        offset += line.len();
        let next = &remainder[offset..];
        let Some(next_line) = next.lines().next() else {
            break;
        };
        if !next_line.is_empty()
            && !next_line.starts_with(' ')
            && !next_line.starts_with('\t')
            && !next_line.starts_with('#')
            && next_line.ends_with(':')
        {
            end = offset;
            break;
        }
    }
    &remainder[..end]
}

// @internal
#[test]
fn rg3_release_lane_is_blocking_and_runs_the_exact_split_ohttp_journey() {
    let job = top_level_job("test:release-rg3");

    assert!(job.contains("allow_failure: false"));
    assert!(job.contains("job: test:smoke"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"merge_request_event\""));
    assert!(job.contains("$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"schedule\""));
    assert!(job.contains(RG3_TEST));
}

// @internal
#[test]
fn rg4_rg5_release_lane_is_blocking_and_runs_the_six_device_journey() {
    let job = top_level_job("test:release-rg4-rg5");

    assert!(job.contains("allow_failure: false"));
    assert!(job.contains("job: test:smoke"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"merge_request_event\""));
    assert!(job.contains("$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"schedule\""));
    assert!(job.contains(RG4_RG5_TEST));
}

// @internal
#[test]
fn native_binary_producer_and_consumers_share_linux_runner() {
    for job_name in [
        "test:smoke",
        "test:release-rg3",
        "test:release-rg4-rg5",
        "test:integration",
    ] {
        let job = top_level_job(job_name);
        assert!(
            job.contains("extends: [.linux-runner,"),
            "{job_name} must run on the Linux platform used for native E2E artifacts"
        );
    }
}
