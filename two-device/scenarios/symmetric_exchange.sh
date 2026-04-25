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
# in parallel through a full QR exchange:
#
#   1. Both devices onboard with their name (parallel).
#   2. Both open Exchange and screenshot their rendered QR (parallel).
#   3. Host decodes both screenshots with `zbarimg` (sequential).
#   4. Each device pastes the peer's QR data via manual entry (parallel).
#   5. Each device asserts the peer is in its Contacts list (parallel).
#   6. Symmetric-invariant check: both must succeed or the harness
#      reports `unilateral completion` (one-sided) or `bilateral failure`.
#
# The orchestrator (`../orchestrator.sh`) handles AVD boot, APK
# install, env-var export, and emulator teardown.
#
# Negative control:
#   `VAUCHI_DEVICE_B_DELAY=<seconds>` inserts a sleep on device B
#   before its onboarding subflow. With a long enough delay the
#   exchange completes one-sided on device A and the harness fails
#   with `unilateral completion` rather than a generic timeout.
#
# Host dependency: `zbarimg` (zbar-tools) for QR decoding.

set -euo pipefail

# ── Paths and inputs ─────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HARNESS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SUBFLOW_DIR="$HARNESS_DIR/subflows"

: "${DEVICE_A_SERIAL:?orchestrator must export DEVICE_A_SERIAL}"
: "${DEVICE_B_SERIAL:?orchestrator must export DEVICE_B_SERIAL}"

DEVICE_B_DELAY="${VAUCHI_DEVICE_B_DELAY:-0}"

QR_A_PNG="/tmp/vauchi_2dev_qr_A.png"
QR_B_PNG="/tmp/vauchi_2dev_qr_B.png"
LOG_DIR="${VAUCHI_2DEV_LOG_DIR:-/tmp/vauchi-2dev-logs}"

# ── Prerequisites ────────────────────────────────────────────

command -v zbarimg >/dev/null 2>&1 || {
    cat >&2 <<'EOF'
symmetric_exchange: zbarimg not in PATH — required to decode QR screenshots.
  Install with:
    macOS:           brew install zbar
    Debian/Ubuntu:   apt install zbar-tools
    Alpine:          apk add zbar
EOF
    exit 2
}
command -v maestro >/dev/null 2>&1 || {
    echo "symmetric_exchange: maestro not in PATH" >&2
    exit 2
}

mkdir -p "$LOG_DIR"
rm -f "$QR_A_PNG" "$QR_B_PNG"

# ── Helpers ──────────────────────────────────────────────────

# Run a Maestro subflow on a specific device, capturing its log
# under $LOG_DIR. Extra `--env KEY=VAL` pairs may follow the
# subflow path. Returns Maestro's exit code on completion.
run_subflow() {
    local serial="$1"
    local subflow="$2"
    local label="$3"
    shift 3

    local log_file="$LOG_DIR/${label}.log"
    if maestro --device "$serial" test "$@" "$SUBFLOW_DIR/$subflow" \
        >"$log_file" 2>&1; then
        return 0
    fi
    echo "  subflow $label failed — see $log_file" >&2
    return 1
}

# Decode a QR screenshot to stdout using zbarimg. Errors out with a
# diagnostic that names the file rather than letting zbar's own
# message pass through unframed.
decode_qr() {
    local png="$1"
    local label="$2"
    local out
    if ! out="$(zbarimg --raw -q "$png" 2>"$LOG_DIR/zbarimg-${label}.log")"; then
        echo "symmetric_exchange: failed to decode QR from $label screenshot ($png)" >&2
        echo "  zbar log: $LOG_DIR/zbarimg-${label}.log" >&2
        exit 1
    fi
    if [[ -z "$out" ]]; then
        echo "symmetric_exchange: $label screenshot decoded to empty string ($png)" >&2
        exit 1
    fi
    printf '%s' "$out"
}

# ── Phase 1: parallel onboarding ─────────────────────────────

echo "→ Phase 1: parallel onboarding (Alice on A, Bob on B)"
run_subflow "$DEVICE_A_SERIAL" "onboard_alice.yaml" "phase1-alice" &
PID_ALICE=$!

(
    if (( DEVICE_B_DELAY > 0 )); then
        echo "  negative control: delaying Bob's onboarding by ${DEVICE_B_DELAY}s"
        sleep "$DEVICE_B_DELAY"
    fi
    run_subflow "$DEVICE_B_SERIAL" "onboard_bob.yaml" "phase1-bob"
) &
PID_BOB=$!

wait "$PID_ALICE" || { echo "Phase 1 FAIL: Alice onboarding failed" >&2; exit 1; }
wait "$PID_BOB"   || { echo "Phase 1 FAIL: Bob onboarding failed" >&2; exit 1; }

# ── Phase 2: parallel QR presentation ────────────────────────

echo "→ Phase 2: parallel QR presentation"
run_subflow "$DEVICE_A_SERIAL" "present_qr.yaml" "phase2-alice" \
    --env "QR_OUT=$QR_A_PNG" &
PID_A=$!
run_subflow "$DEVICE_B_SERIAL" "present_qr.yaml" "phase2-bob" \
    --env "QR_OUT=$QR_B_PNG" &
PID_B=$!

wait "$PID_A" || { echo "Phase 2 FAIL: Alice QR present failed" >&2; exit 1; }
wait "$PID_B" || { echo "Phase 2 FAIL: Bob QR present failed" >&2; exit 1; }

[[ -f "$QR_A_PNG" ]] || { echo "Phase 2 FAIL: $QR_A_PNG not written" >&2; exit 1; }
[[ -f "$QR_B_PNG" ]] || { echo "Phase 2 FAIL: $QR_B_PNG not written" >&2; exit 1; }

# ── Phase 3: decode QR screenshots ───────────────────────────

echo "→ Phase 3: decode QR screenshots"
QR_A_DATA="$(decode_qr "$QR_A_PNG" "alice")"
QR_B_DATA="$(decode_qr "$QR_B_PNG" "bob")"
echo "  Alice QR: ${#QR_A_DATA} bytes"
echo "  Bob QR:   ${#QR_B_DATA} bytes"

# ── Phase 4: parallel cross-feed scan ────────────────────────

echo "→ Phase 4: parallel scan (Alice scans Bob's QR; Bob scans Alice's QR)"
run_subflow "$DEVICE_A_SERIAL" "scan_qr_manual.yaml" "phase4-alice" \
    --env "QR_DATA=$QR_B_DATA" &
PID_A=$!
run_subflow "$DEVICE_B_SERIAL" "scan_qr_manual.yaml" "phase4-bob" \
    --env "QR_DATA=$QR_A_DATA" &
PID_B=$!

# Both halves of the exchange should complete. If either fails the
# subflow Maestro returns non-zero — that's the "unilateral
# completion" signal we want to surface explicitly in Phase 6
# rather than leak through here.
SCAN_A_RC=0; wait "$PID_A" || SCAN_A_RC=$?
SCAN_B_RC=0; wait "$PID_B" || SCAN_B_RC=$?

# ── Phase 5: parallel symmetric-contacts assertion ───────────

echo "→ Phase 5: parallel contacts assertion"
ASSERT_A_RC=0
ASSERT_B_RC=0

if (( SCAN_A_RC == 0 )); then
    run_subflow "$DEVICE_A_SERIAL" "assert_has_bob.yaml" "phase5-alice" &
    PID_A=$!
fi
if (( SCAN_B_RC == 0 )); then
    run_subflow "$DEVICE_B_SERIAL" "assert_has_alice.yaml" "phase5-bob" &
    PID_B=$!
fi

if (( SCAN_A_RC == 0 )); then
    wait "$PID_A" || ASSERT_A_RC=$?
else
    ASSERT_A_RC=$SCAN_A_RC
fi
if (( SCAN_B_RC == 0 )); then
    wait "$PID_B" || ASSERT_B_RC=$?
else
    ASSERT_B_RC=$SCAN_B_RC
fi

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
