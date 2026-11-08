#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The release lanes run on a merge request only when it changes what they
# exercise, and every lane on main, the nightly and tags, which is where
# the release evidence is read (vauchi/private#1032, decision D6). A lane
# whose MR trigger misses its own test file, or a shared input, lets a
# change that breaks it merge, and main finds it one pipeline later.
#
# For each lane this checks that its merge-request rule lists:
# - every test module file its nextest filter names (tests/it/<name>.rs);
# - every shared input: the harness, the helper test modules, manifests,
#   lockfile and toolchain, nextest config, the pinned set and its
#   scripts, and this CI file;
# and that its main and schedule rules carry no `changes`.
#
# Usage: ci/tests/lane-paths.test.sh [ci-config]   (from the e2e repo root)

set -eu
set -f

ci_config=${1:-.gitlab-ci.yml}
failures=0
fail() {
    echo "FAIL: $1" >&2
    failures=$((failures + 1))
}

LANES="test:backup-restore test:release-rg3 test:release-rg4-rg5
test:release-rg6-observability test:release-rg7 test:release-rg8
test:release-rg10 test:release-rg15 test:tui-journey"

SHARED="src/**/* tests/it/main.rs tests/it/ohttp_helpers.rs tests/it/six_device.rs
tests/*.rs tests/fixtures/**/* Cargo.toml Cargo.lock rust-toolchain.toml .config/**/*
ci/pins.env ci/build-pinned-binaries.sh ci/point-core-at.sh ci/check-core-pin.sh
ci/check-previous-core-pin.sh
.gitlab-ci.yml"

job_block() {
    awk -v job="$1" '
        $0 == job ":" { in_job = 1; next }
        in_job && /^[^[:space:]#]/ { exit }
        in_job { print }
    ' "$ci_config"
}

# The rule list of a job: from `rules:` to the next key at its indent.
rules_of() {
    printf '%s\n' "$1" | awk '
        /^  rules:/ { in_rules = 1; next }
        in_rules && /^  [a-z_]+:/ { exit }
        in_rules { print }
    '
}

for lane in $LANES; do
    lane_start=$failures
    block=$(job_block "$lane")
    if [ -z "$block" ]; then
        fail "$lane is not in $ci_config"
        continue
    fi
    rules=$(rules_of "$block")
    mr_rule=$(printf '%s\n' "$rules" | awk '
        /^    - if:/ { in_mr = ($0 ~ /merge_request_event|CI_MERGE_REQUEST_IID/) }
        in_mr { print }
    ')
    if [ -z "$mr_rule" ]; then
        fail "$lane has no merge-request rule"
        continue
    fi
    printf '%s\n' "$mr_rule" | grep -q 'changes:' || {
        fail "$lane runs on every merge request; give its rule the paths it exercises"
        continue
    }

    filter=$(printf '%s\n' "$block" | grep -E '^ +-E ' || true)
    modules=""
    for token in $(printf '%s\n' "$filter" | grep -oE '[a-z_]{3,}' | sort -u); do
        [ -f "tests/it/$token.rs" ] && modules="$modules tests/it/$token.rs"
    done
    [ -n "$modules" ] || fail "$lane's filter names no tests/it module this test can find"

    listed=$(printf '%s\n' "$mr_rule" | sed -n 's/^ *- *"\{0,1\}\([^"]*\)"\{0,1\} *$/\1/p')
    for path in $SHARED $modules; do
        if printf '%s\n' "$listed" | grep -qxF -- "$path"; then
            :
        else
            fail "$lane's merge-request rule does not trigger on $path"
        fi
    done

    other=$(printf '%s\n' "$rules" | awk '
        /^    - if:/ { in_other = ($0 !~ /merge_request_event|CI_MERGE_REQUEST_IID/) }
        in_other { print }
    ')
    if printf '%s\n' "$other" | grep -q 'changes:'; then
        fail "$lane's main or schedule rule has changes; every lane runs in full there"
    elif [ "$failures" -eq "$lane_start" ]; then
        echo "PASS: $lane runs by path on merge requests and in full elsewhere"
    fi
done

smoke_mr=$(rules_of "$(job_block test:smoke)" | grep -A1 'merge_request_event' || true)
if printf '%s\n' "$smoke_mr" | grep -q 'changes:'; then
    fail "test:smoke must stay on every merge request"
fi

if [ "$failures" -gt 0 ]; then
    echo "$failures failure(s)" >&2
    exit 1
fi
echo "OK: every release lane triggers on what it exercises"
