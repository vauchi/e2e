#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The release lanes compile in under a second and then test for up to half
# an hour (rg4-rg5: 0.42 s build, 28.5 min of six-device tests, which are
# latency-bound chains of CLI calls). In the single cold-rebuild-tree slot
# one MR's lanes ran one after another, ~65 min, behind main's. They run in
# two groups instead, the maintainer's cap of two at a time
# (vauchi/private#1179): e2e-lane-a holds rg4-rg5, e2e-lane-b the rest.
# Jobs that compile the tree keep cold-rebuild-tree.
#
# Usage: ci/tests/lane-groups.test.sh [ci-config]   (from the e2e repo root)

set -eu

ci_config=${1:-.gitlab-ci.yml}
failures=0
fail() {
    echo "FAIL: $1" >&2
    failures=$((failures + 1))
}

job_block() {
    awk -v job="$1" '
        $0 == job ":" { in_job = 1; next }
        in_job && /^[^[:space:]#]/ { exit }
        in_job { print }
    ' "$ci_config"
}

expect_group() { # job group
    block=$(job_block "$1")
    if [ -z "$block" ]; then
        fail "$1 is not in $ci_config"
        return
    fi
    if printf '%s\n' "$block" | grep -q 'timeout-cold-rebuild'; then
        fail "$1 still takes the cold-rebuild-tree slot"
    fi
    if printf '%s\n' "$block" | grep -qx "  resource_group: $2"; then
        echo "PASS: $1 runs in $2"
    else
        fail "$1 must set resource_group: $2"
    fi
}

expect_cold_rebuild() { # job
    if job_block "$1" | grep -q 'timeout-cold-rebuild'; then
        echo "PASS: $1 keeps cold-rebuild-tree"
    else
        fail "$1 compiles the tree and must keep .timeout-cold-rebuild"
    fi
}

expect_group test:release-rg4-rg5 e2e-lane-a
for lane in test:release-rg10 test:release-rg15 test:release-rg3 \
    test:release-rg6-observability test:release-rg7 test:release-rg8 \
    test:backup-restore; do
    expect_group "$lane" e2e-lane-b
done
for job in test:smoke test:tui-journey test:integration lint:clippy; do
    expect_cold_rebuild "$job"
done

if [ "$failures" -gt 0 ]; then
    echo "$failures failure(s)" >&2
    exit 1
fi
echo "OK: release lanes run two at a time, builds keep their slot"
