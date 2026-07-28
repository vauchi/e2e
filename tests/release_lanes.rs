// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

const CI_CONFIG: &str = include_str!("../.gitlab-ci.yml");
const RG3_TEST: &str =
    "orchestrator_default_ohttp::integration_ohttp_split_relay_config_routes_via_ohttp_relay";
const RG4_RG5_TEST_FILTER: &str = "multi_device_sync::integration_six_device_(exchange_and_update_convergence|single_exchange_convergence|offline_catchup_converges_exact_values|faulted_relay_delivery_converges_exact_values|duplicate_ohttp_delivery_converges_exact_values|concurrent_field_edits_converge|bounded_clock_skew_converges_to_later_update|personal_note_tombstone_converges_owner_only|replacement_and_revocation_preserve_active_convergence|lost_primary_continuity_certification)";
const OHTTP_E2E_FAULT_BUILD: &str = "cargo build --release --features e2e-faults --manifest-path \"$BUILD_TMPDIR/ohttp-relay/Cargo.toml\"";
const OHTTP_E2E_FAULT_PROFILE: &str = "OHTTP_BUILD_PROFILE=\"e2e-faults-v1\"";
const RG6_TEST: &str =
    "ohttp_integration::integration_ohttp_relay_observations_exclude_update_content";
const RG8_TEST: &str = "ohttp_fail_closed_matrix";

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
fn scheduled_pipeline_requires_explicit_e2e_ownership() {
    let workflow = top_level_job("workflow");
    let selected = "$CI_PIPELINE_SOURCE == \"schedule\" && $SCHEDULE_KIND == \"e2e\"";
    let reject_other_schedules = "$CI_PIPELINE_SOURCE == \"schedule\"\n      when: never";

    assert!(workflow.contains(selected));
    assert!(workflow.contains(reject_other_schedules));
    assert!(workflow.find(selected) < workflow.find(reject_other_schedules));
}

// @internal
#[test]
fn scheduled_pipeline_does_not_publish_pages_or_mirror() {
    for job_name in ["pages", "github-mirror"] {
        let job = top_level_job(job_name);
        assert!(job.contains("$CI_PIPELINE_SOURCE == \"schedule\""));
        assert!(job.contains("when: never"));
    }
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
    assert!(job.contains(RG4_RG5_TEST_FILTER));
}

// @internal
#[test]
fn native_e2e_binary_builds_the_feature_gated_ohttp_fault_controller() {
    assert_eq!(CI_CONFIG.matches(OHTTP_E2E_FAULT_BUILD).count(), 2);
}

// @internal
#[test]
fn ohttp_fault_build_profile_invalidates_incompatible_cached_binaries() {
    let smoke = top_level_job("test:smoke");
    assert!(smoke.contains(OHTTP_E2E_FAULT_PROFILE));
    assert!(smoke.contains(".smoke-bin/ohttp.build-profile"));
    assert!(smoke.contains("echo \"$OHTTP_BUILD_PROFILE\" > .smoke-bin/ohttp.build-profile"));

    let integration = top_level_job("test:integration");
    assert!(integration.contains(OHTTP_E2E_FAULT_PROFILE));
    assert!(integration.contains("$E2E_BIN_DIR/ohttp.build-profile"));
    assert!(
        integration
            .contains("echo \"$OHTTP_BUILD_PROFILE\" > \"$E2E_BIN_DIR/ohttp.build-profile\"")
    );
}

// @internal
#[test]
fn rg6_release_lane_is_blocking_and_runs_the_relay_observer() {
    let job = top_level_job("test:release-rg6-observability");

    assert!(job.contains("allow_failure: false"));
    assert!(job.contains("job: test:smoke"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"merge_request_event\""));
    assert!(job.contains("$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"schedule\""));
    assert!(job.contains(RG6_TEST));
}

// @internal
#[test]
fn rg8_release_lane_is_blocking_and_runs_the_fail_closed_matrix() {
    let job = top_level_job("test:release-rg8");

    assert!(job.contains("allow_failure: false"));
    assert!(job.contains("job: test:smoke"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"merge_request_event\""));
    assert!(job.contains("$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH"));
    assert!(job.contains("$CI_PIPELINE_SOURCE == \"schedule\""));
    assert!(job.contains(RG8_TEST));
}

// @internal
#[test]
fn native_binary_producer_and_consumers_share_linux_runner() {
    for job_name in [
        "test:smoke",
        "test:release-rg3",
        "test:release-rg4-rg5",
        "test:release-rg6-observability",
        "test:release-rg8",
        "test:integration",
    ] {
        let job = top_level_job(job_name);
        assert!(
            job.contains("extends: [.linux-runner,"),
            "{job_name} must run on the Linux platform used for native E2E artifacts"
        );
    }
}
