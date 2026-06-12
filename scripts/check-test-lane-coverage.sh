#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fail if any non-ignored test in the `vauchi-e2e-tests::it` integration
# suite is selected by no CI run lane. Guards the root cause of problem
# 2026-06-11-e2e-tests-invisible-to-ci: CI picks integration tests by
# name filter, so a test that happens to match no filter compiles, passes
# locally, and silently never runs in any pipeline.
#
# Coverage = the integration lane filter (ci/integration-test-filter),
# which is the comprehensive lane and is expected to select every
# non-ignored ::it test. Tests deliberately excluded go in
# ci/test-coverage-allowlist.txt with a reason.
#
# Listing is pluggable via LANE_LIST_CMD so the logic is unit-testable
# without compiling the crate (see check-test-lane-coverage.test.sh).
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_DIR=$(dirname -- "$SCRIPT_DIR")
FILTER_FILE="${LANE_FILTER_FILE:-$REPO_DIR/ci/integration-test-filter}"
ALLOWLIST_FILE="${LANE_ALLOWLIST_FILE:-$REPO_DIR/ci/test-coverage-allowlist.txt}"
SUITE="vauchi-e2e-tests::it"

# Echo the `module::test` names selected by a nextest -E expression ($1),
# restricted to the integration suite. Default lister shells out to
# nextest; LANE_LIST_CMD overrides it (the test injects fixtures).
list_tests() {
	if [ -n "${LANE_LIST_CMD:-}" ]; then
		"$LANE_LIST_CMD" "$1"
	else
		( cd "$REPO_DIR" && cargo nextest list --profile ci -E "$1" 2>/dev/null ) \
			| awk -v s="$SUITE" '$1 == s { print $2 }'
	fi
}

FILTER=$(cat "$FILTER_FILE")

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

list_tests 'kind(test)' | sort -u >"$TMP/universe"
list_tests "$FILTER" | sort -u >"$TMP/selected"
# Allowlist counts as covered. Strip comments/blank lines.
{ grep -vE '^[[:space:]]*(#|$)' "$ALLOWLIST_FILE" 2>/dev/null || true; } | sort -u >"$TMP/allow"
sort -u "$TMP/selected" "$TMP/allow" >"$TMP/covered"

# uncovered = universe \ covered  (portable; no process substitution).
awk 'NR==FNR { c[$0]=1; next } !($0 in c)' "$TMP/covered" "$TMP/universe" >"$TMP/uncovered"

universe_n=$(grep -c . "$TMP/universe" || true)

if [ -s "$TMP/uncovered" ]; then
	echo "ERROR: ${SUITE} tests selected by no CI lane:" >&2
	sed 's/^/  - /' "$TMP/uncovered" >&2
	echo >&2
	echo "Each compiles and passes locally but runs in NO pipeline." >&2
	echo "Fix: make ci/integration-test-filter select them, or add to" >&2
	echo "ci/test-coverage-allowlist.txt with a reason." >&2
	echo "(problem 2026-06-11-e2e-tests-invisible-to-ci)" >&2
	exit 1
fi

echo "OK: all ${universe_n} non-ignored ${SUITE} tests are covered by a CI lane."
