#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fail if the e2e stall guards are removed or weakened. Guards the fix
# for problem 2026-07-19-e2e-multi-device-serialization-hang: runner
# contention inflated every test 10-15x and, with no per-test timeout,
# lanes died as inscrutable `job_token_expired` / 45-min kills instead
# of named, debuggable failures. These three guards are the
# non-regressable part of that fix — removing any of them must turn CI
# red here, loudly, instead of silently re-opening the failure mode:
#
#   1. nextest.toml keeps slow-timeout + terminate-after (both profiles)
#   2. device/cli.rs keeps kill_on_drop(true) on the shared CLI runner
#   3. orchestrator.rs never reintroduces the unbounded `reqwest::get(`
set -eu

REPO_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
NEXTEST="$REPO_DIR/.config/nextest.toml"
CLI="$REPO_DIR/src/device/cli.rs"
ORCH="$REPO_DIR/src/orchestrator.rs"
ERRORS=0

fail() {
    echo "  MISSING: $1"
    ERRORS=$((ERRORS + 1))
}

grep -q "slow-timeout" "$NEXTEST" \
    || fail "$NEXTEST: slow-timeout (per-test budget) removed"
grep -q "terminate-after" "$NEXTEST" \
    || fail "$NEXTEST: terminate-after removed — nextest reports SLOW but never kills"
grep -q "kill_on_drop(true)" "$CLI" \
    || fail "$CLI: kill_on_drop(true) removed — timed-out CLI children leak again"
if grep -q "reqwest::get(" "$ORCH"; then
    fail "$ORCH: unbounded reqwest::get( reintroduced — use the bounded client"
fi

if [ "$ERRORS" -gt 0 ]; then
    echo ""
    echo "FAILED: $ERRORS stall guard(s) regressed — see"
    echo "  problems/2026-07-19-e2e-multi-device-serialization-hang"
    exit 1
fi
echo "Test stall guards OK"
