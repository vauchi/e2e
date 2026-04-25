#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

# symmetric_exchange.sh — Layer 4 regression test for
# `_private/docs/problems/2026-03-31-unilateral-exchange-race`.
#
# Plan: `_private/docs/planning/todo/2026-04-20-frontend-correctness-strategy-plan.md`
# Phase 4 Task 4.2 Steps 2-4.
#
# Drives Alice on `$DEVICE_A_SERIAL` and Bob on `$DEVICE_B_SERIAL`
# in parallel through a full QR exchange and asserts that both
# devices end up with the peer's contact card. A one-sided
# completion fails with `unilateral completion`; a both-sides
# failure fails with `bilateral failure` — both are distinct from
# a generic timeout.
#
# Phase logic lives in `../lib/two_device_lib.sh::establish_exchange`;
# this scenario's contribution is the verdict block.
#
# Negative control:
#   `VAUCHI_DEVICE_B_DELAY=<seconds>` inserts a sleep on device B
#   before its onboarding subflow. With a long enough delay the
#   exchange completes one-sided on device A and the verdict
#   surfaces as `unilateral completion`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/two_device_lib.sh
source "$SCRIPT_DIR/../lib/two_device_lib.sh"

command -v zbarimg >/dev/null 2>&1 || {
    echo "symmetric_exchange: zbarimg not in PATH (orchestrator should have caught this)" >&2
    exit 2
}
command -v maestro >/dev/null 2>&1 || {
    echo "symmetric_exchange: maestro not in PATH" >&2
    exit 2
}

establish_exchange

# ── Phase 6: symmetric-invariant verdict ─────────────────────

echo "→ Phase 6: symmetric-invariant verdict"
A_OK=$(( ASSERT_A_RC == 0 ))
B_OK=$(( ASSERT_B_RC == 0 ))

if (( A_OK == 1 && B_OK == 1 )); then
    echo ""
    echo "PASS: symmetric exchange — both devices hold the peer's contact"
    exit 0
fi

if (( A_OK == 0 && B_OK == 0 )); then
    echo ""
    echo "FAIL: bilateral failure — neither device has the peer's contact"
    echo "  Alice (device A) Contacts assertion exit: $ASSERT_A_RC"
    echo "  Bob   (device B) Contacts assertion exit: $ASSERT_B_RC"
    echo "  Logs: $LOG_DIR/phase{4,5}-{alice,bob}.log"
    exit 1
fi

echo ""
echo "FAIL: unilateral completion — exchange landed on one side only"
if (( A_OK == 1 )); then
    echo "  device A (Alice) holds Bob ✓"
    echo "  device B (Bob)   missing Alice ✗ (assert exit $ASSERT_B_RC)"
else
    echo "  device A (Alice) missing Bob ✗ (assert exit $ASSERT_A_RC)"
    echo "  device B (Bob)   holds Alice ✓"
fi
echo "  Logs: $LOG_DIR/phase{4,5}-{alice,bob}.log"
exit 1
