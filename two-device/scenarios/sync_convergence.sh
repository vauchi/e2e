#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

# sync_convergence.sh — Layer 4 cross-device sync gate.
#
# Plan: `_private/docs/planning/todo/2026-04-20-frontend-correctness-strategy-plan.md`
# Phase 4 Task 4.3.
#
# Establishes the symmetric-exchange precondition (both devices
# hold each other's contact, via the shared library helper),
# then drives Alice through a card edit and verifies Bob's device
# observes the new name within a bounded sync window.
#
# Verdict semantics:
#   PASS              — Bob sees the new name within the window.
#   sync diverged     — Bob still sees the OLD name. Update did
#                       not propagate (or was dropped). This is
#                       the regression target for sync delivery
#                       failures.
#   contact lost      — Bob sees neither old nor new name. The
#                       contact entry is gone entirely; sync
#                       deletion bug.
#   precondition failed — Phases 1-5 of the exchange did not
#                       complete (use symmetric_exchange.sh to
#                       triage that class of bug).
#
# Tunables:
#   VAUCHI_SYNC_WINDOW_S   — wait bound, seconds. Default 30.
#   VAUCHI_DEVICE_B_DELAY  — passes through to establish_exchange.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/two_device_lib.sh
source "$SCRIPT_DIR/../lib/two_device_lib.sh"

command -v zbarimg >/dev/null 2>&1 || {
    echo "sync_convergence: zbarimg not in PATH" >&2
    exit 2
}
command -v maestro >/dev/null 2>&1 || {
    echo "sync_convergence: maestro not in PATH" >&2
    exit 2
}

OLD_NAME="Alice"
NEW_NAME="Alice v2"
SYNC_WINDOW_S="${VAUCHI_SYNC_WINDOW_S:-30}"
SYNC_WINDOW_MS=$(( SYNC_WINDOW_S * 1000 ))

# ── Phases 1–5: establish symmetric exchange precondition ────

establish_exchange

if (( ASSERT_A_RC != 0 || ASSERT_B_RC != 0 )); then
    echo ""
    echo "FAIL: precondition failed — symmetric exchange did not complete"
    echo "  Alice (device A) Contacts assertion exit: $ASSERT_A_RC"
    echo "  Bob   (device B) Contacts assertion exit: $ASSERT_B_RC"
    echo "  Run symmetric_exchange.sh to triage this class of bug."
    echo "  Logs: $LOG_DIR/phase{1,2,4,5}-{alice,bob}.log"
    exit 1
fi

# ── Phase 6: Alice edits her display name ────────────────────

echo "→ Phase 6: Alice edits display name → '$NEW_NAME'"
if ! run_subflow "$DEVICE_A_SERIAL" "alice_edit_display_name.yaml" "phase6-alice" \
    --env "NEW_NAME=$NEW_NAME"; then
    echo ""
    echo "FAIL: Alice could not save the card update"
    echo "  Logs: $LOG_DIR/phase6-alice.log"
    exit 1
fi

# ── Phase 7: Bob waits for the update ────────────────────────

echo "→ Phase 7: Bob waits for '$NEW_NAME' (window ${SYNC_WINDOW_S}s)"
OBSERVE_RC=0
run_subflow "$DEVICE_B_SERIAL" "bob_observe_updated_name.yaml" "phase7-bob" \
    --env "NEW_NAME=$NEW_NAME" \
    --env "SYNC_WINDOW_MS=$SYNC_WINDOW_MS" \
    || OBSERVE_RC=$?

# ── Phase 8: verdict ─────────────────────────────────────────

echo "→ Phase 8: convergence verdict"
if (( OBSERVE_RC == 0 )); then
    echo ""
    echo "PASS: sync converged — Bob observed '$NEW_NAME' within ${SYNC_WINDOW_S}s"
    exit 0
fi

# Update did not arrive. Disambiguate divergence (old name still
# present) vs contact loss (neither name present) by asserting
# on the OLD name.
echo "  observation timed out; running diagnosis subflow"
DIAGNOSE_RC=0
run_subflow "$DEVICE_B_SERIAL" "bob_diagnose_no_update.yaml" "phase8-bob-diagnose" \
    --env "OLD_NAME=$OLD_NAME" \
    || DIAGNOSE_RC=$?

echo ""
if (( DIAGNOSE_RC == 0 )); then
    echo "FAIL: sync diverged — Bob still sees '$OLD_NAME', not '$NEW_NAME'"
    echo "  Alice's local card was saved (Phase 6 passed)."
    echo "  Update did not propagate within the ${SYNC_WINDOW_S}s window."
    echo "  This is the sync-delivery regression target."
else
    echo "FAIL: contact lost — Bob sees neither '$OLD_NAME' nor '$NEW_NAME'"
    echo "  Catastrophic regression: sync deleted the contact entry."
fi
echo "  Logs: $LOG_DIR/phase{6,7,8}-*.log"
exit 1
