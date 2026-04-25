# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# shellcheck shell=bash
#
# two_device_lib.sh — shared helpers for `e2e/two-device/scenarios/*.sh`.
#
# Sourced (not executed) by scenario scripts. Provides:
#
#   run_subflow <serial> <subflow.yaml> <log-label> [--env K=V ...]
#       Runs a single Maestro subflow against a single device,
#       logging stdout+stderr to "$LOG_DIR/<log-label>.log".
#
#   decode_qr <png> <label>
#       Wraps `zbarimg --raw` with a clear failure message that
#       names the file rather than letting zbar's bare error
#       string surface unframed. Exits 1 on decode failure.
#
#   establish_exchange
#       Drives Phases 1–5 of the symmetric-exchange precondition:
#       parallel onboarding, parallel QR present, host-side decode,
#       parallel cross-feed scan, parallel "peer is in Contacts"
#       assertion. Exits 1 with `precondition failed: <reason>` if
#       any step fails — caller scenarios should treat that as a
#       distinct signal from their own assertion failures.
#
# Required env (set by orchestrator.sh):
#   - DEVICE_A_SERIAL, DEVICE_B_SERIAL
#
# Library-set globals available to callers after sourcing:
#   - SUBFLOW_DIR        — absolute path to two-device/subflows/
#   - LOG_DIR            — Maestro per-subflow log directory
#   - QR_A_PNG, QR_B_PNG — paths to each device's QR screenshot

# Resolve paths relative to the lib file (callers source from
# anywhere — don't trust their CWD).
__TWO_DEVICE_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HARNESS_DIR="$(cd "$__TWO_DEVICE_LIB_DIR/.." && pwd)"
SUBFLOW_DIR="$HARNESS_DIR/subflows"
LOG_DIR="${VAUCHI_2DEV_LOG_DIR:-/tmp/vauchi-2dev-logs}"
QR_A_PNG="/tmp/vauchi_2dev_qr_A.png"
QR_B_PNG="/tmp/vauchi_2dev_qr_B.png"

mkdir -p "$LOG_DIR"

# ── run_subflow ──────────────────────────────────────────────

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

# ── decode_qr ────────────────────────────────────────────────

decode_qr() {
    local png="$1"
    local label="$2"
    local out
    if ! out="$(zbarimg --raw -q "$png" 2>"$LOG_DIR/zbarimg-${label}.log")"; then
        echo "two_device_lib: failed to decode QR from $label screenshot ($png)" >&2
        echo "  zbar log: $LOG_DIR/zbarimg-${label}.log" >&2
        exit 1
    fi
    if [[ -z "$out" ]]; then
        echo "two_device_lib: $label screenshot decoded to empty string ($png)" >&2
        exit 1
    fi
    printf '%s' "$out"
}

# ── establish_exchange ───────────────────────────────────────

establish_exchange() {
    : "${DEVICE_A_SERIAL:?establish_exchange: orchestrator must export DEVICE_A_SERIAL}"
    : "${DEVICE_B_SERIAL:?establish_exchange: orchestrator must export DEVICE_B_SERIAL}"
    local device_b_delay="${VAUCHI_DEVICE_B_DELAY:-0}"

    rm -f "$QR_A_PNG" "$QR_B_PNG"

    # Phase 1 — parallel onboarding.
    echo "→ Phase 1: parallel onboarding (Alice on A, Bob on B)"
    run_subflow "$DEVICE_A_SERIAL" "onboard_alice.yaml" "phase1-alice" &
    local pid_alice=$!
    (
        if (( device_b_delay > 0 )); then
            echo "  negative control: delaying Bob's onboarding by ${device_b_delay}s"
            sleep "$device_b_delay"
        fi
        run_subflow "$DEVICE_B_SERIAL" "onboard_bob.yaml" "phase1-bob"
    ) &
    local pid_bob=$!
    wait "$pid_alice" || { echo "precondition failed: Alice onboarding" >&2; exit 1; }
    wait "$pid_bob"   || { echo "precondition failed: Bob onboarding"   >&2; exit 1; }

    # Phase 2 — parallel QR presentation.
    echo "→ Phase 2: parallel QR presentation"
    run_subflow "$DEVICE_A_SERIAL" "present_qr.yaml" "phase2-alice" \
        --env "QR_OUT=$QR_A_PNG" &
    local pid_a=$!
    run_subflow "$DEVICE_B_SERIAL" "present_qr.yaml" "phase2-bob" \
        --env "QR_OUT=$QR_B_PNG" &
    local pid_b=$!
    wait "$pid_a" || { echo "precondition failed: Alice QR present" >&2; exit 1; }
    wait "$pid_b" || { echo "precondition failed: Bob QR present"   >&2; exit 1; }
    [[ -f "$QR_A_PNG" ]] || { echo "precondition failed: $QR_A_PNG missing" >&2; exit 1; }
    [[ -f "$QR_B_PNG" ]] || { echo "precondition failed: $QR_B_PNG missing" >&2; exit 1; }

    # Phase 3 — host-side decode.
    echo "→ Phase 3: decode QR screenshots"
    QR_A_DATA="$(decode_qr "$QR_A_PNG" "alice")"
    QR_B_DATA="$(decode_qr "$QR_B_PNG" "bob")"
    echo "  Alice QR: ${#QR_A_DATA} bytes"
    echo "  Bob QR:   ${#QR_B_DATA} bytes"

    # Phase 4 — parallel cross-feed scan.
    echo "→ Phase 4: parallel scan (Alice scans Bob's QR; Bob scans Alice's QR)"
    run_subflow "$DEVICE_A_SERIAL" "scan_qr_manual.yaml" "phase4-alice" \
        --env "QR_DATA=$QR_B_DATA" &
    pid_a=$!
    run_subflow "$DEVICE_B_SERIAL" "scan_qr_manual.yaml" "phase4-bob" \
        --env "QR_DATA=$QR_A_DATA" &
    pid_b=$!
    SCAN_A_RC=0; wait "$pid_a" || SCAN_A_RC=$?
    SCAN_B_RC=0; wait "$pid_b" || SCAN_B_RC=$?
    export SCAN_A_RC SCAN_B_RC

    # Phase 5 — parallel symmetric-contacts assertion.
    echo "→ Phase 5: parallel contacts assertion"
    ASSERT_A_RC=0
    ASSERT_B_RC=0
    if (( SCAN_A_RC == 0 )); then
        run_subflow "$DEVICE_A_SERIAL" "assert_has_bob.yaml" "phase5-alice" &
        pid_a=$!
    fi
    if (( SCAN_B_RC == 0 )); then
        run_subflow "$DEVICE_B_SERIAL" "assert_has_alice.yaml" "phase5-bob" &
        pid_b=$!
    fi
    if (( SCAN_A_RC == 0 )); then
        wait "$pid_a" || ASSERT_A_RC=$?
    else
        ASSERT_A_RC=$SCAN_A_RC
    fi
    if (( SCAN_B_RC == 0 )); then
        wait "$pid_b" || ASSERT_B_RC=$?
    else
        ASSERT_B_RC=$SCAN_B_RC
    fi
    export ASSERT_A_RC ASSERT_B_RC
}
