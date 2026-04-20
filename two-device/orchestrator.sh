#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

# orchestrator.sh — Two-device protocol harness driver.
#
# Plan: _private/docs/planning/todo/2026-04-20-frontend-correctness-strategy-plan.md
# Phase 4 Task 4.1. Layer 4 of the frontend correctness strategy.
#
# Boots two Android emulators in parallel, installs the debug APK
# on both, then invokes a scenario shell script that drives the
# two devices via `adb -s <serial>` and Maestro subflows. Cleans
# up both emulators on exit via a trap.
#
# This is the scaffold. The symmetric-exchange scenario starts as
# a stub; its actual protocol driving (QR presentation on A,
# scan on B, then reverse) is authored inside
# `scenarios/symmetric_exchange.yaml`.
#
# Usage:
#   e2e/two-device/orchestrator.sh <scenario_name>
#
# Scenarios live in `e2e/two-device/scenarios/<name>.yaml` and
# may reference the exposed env vars:
#   - DEVICE_A_SERIAL — first emulator's adb serial
#   - DEVICE_B_SERIAL — second emulator's adb serial
#   - DEVICE_A_AVD    — first emulator's AVD name
#   - DEVICE_B_AVD    — second emulator's AVD name
#   - APK_PATH        — path to the debug APK installed on both
#
# Config via env (pre-running):
#   - VAUCHI_AVD_A       (default: vauchi-test-0)
#   - VAUCHI_AVD_B       (default: vauchi-test-1)
#   - VAUCHI_APK_PATH    (default: android/app/build/outputs/apk/debug/app-debug.apk)
#   - VAUCHI_BOOT_TIMEOUT_S (default: 120)

set -euo pipefail

SCENARIO="${1:-}"
if [[ -z "$SCENARIO" ]]; then
    printf 'usage: %s <scenario_name>\n' "$0" >&2
    printf '       scenarios available under e2e/two-device/scenarios/\n' >&2
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SCENARIO_FILE="$SCRIPT_DIR/scenarios/${SCENARIO}.yaml"

if [[ ! -f "$SCENARIO_FILE" ]]; then
    printf 'orchestrator: scenario not found: %s\n' "$SCENARIO_FILE" >&2
    exit 2
fi

AVD_A="${VAUCHI_AVD_A:-vauchi-test-0}"
AVD_B="${VAUCHI_AVD_B:-vauchi-test-1}"
APK_PATH="${VAUCHI_APK_PATH:-$WORKSPACE_ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
BOOT_TIMEOUT_S="${VAUCHI_BOOT_TIMEOUT_S:-120}"

# ── Prerequisites ────────────────────────────────────────────

command -v adb       >/dev/null || { echo "orchestrator: adb not in PATH"; exit 2; }
command -v emulator  >/dev/null || { echo "orchestrator: emulator not in PATH"; exit 2; }
command -v maestro   >/dev/null || { echo "orchestrator: maestro not in PATH"; exit 2; }

[[ -f "$APK_PATH" ]] || {
    echo "orchestrator: APK not found at $APK_PATH"
    echo "  build with: just rebuild-android"
    exit 2
}

# Verify AVDs exist.
for avd in "$AVD_A" "$AVD_B"; do
    if ! emulator -list-avds 2>/dev/null | grep -q "^${avd}$"; then
        echo "orchestrator: AVD '$avd' not found"
        echo "  available AVDs:" >&2
        emulator -list-avds | sed 's/^/    /' >&2
        echo ""
        echo "  create with Android Studio or 'avdmanager create avd ...'" >&2
        exit 2
    fi
done

# ── Emulator boot ────────────────────────────────────────────

# Pick two free console ports well above the default 5554 range
# to avoid collisions with any ambient emulators the developer
# left running. Emulator ports are even; the adb serial is
# `emulator-<port>`.
PORT_A=5600
PORT_B=5602
SERIAL_A="emulator-${PORT_A}"
SERIAL_B="emulator-${PORT_B}"

start_emulator() {
    local avd="$1"
    local port="$2"
    echo "→ Starting AVD '$avd' on port $port ..."
    emulator -avd "$avd" -port "$port" -no-snapshot-save -no-window >"/tmp/vauchi-emu-${port}.log" 2>&1 &
    echo "$!"
}

wait_for_device() {
    local serial="$1"
    local timeout="$2"
    local elapsed=0
    echo "  waiting for $serial to report boot_completed ..."
    while (( elapsed < timeout )); do
        if adb -s "$serial" shell getprop sys.boot_completed 2>/dev/null | grep -q '^1$'; then
            echo "  $serial ready"
            return 0
        fi
        sleep 2
        elapsed=$((elapsed + 2))
    done
    echo "orchestrator: $serial did not boot within ${timeout}s" >&2
    return 1
}

cleanup() {
    echo ""
    echo "→ Cleaning up emulators ..."
    adb -s "$SERIAL_A" emu kill 2>/dev/null || true
    adb -s "$SERIAL_B" emu kill 2>/dev/null || true
    # Give adb a moment to drain, then report.
    sleep 2
    echo "  done"
}
trap cleanup EXIT INT TERM

PID_A=$(start_emulator "$AVD_A" "$PORT_A")
PID_B=$(start_emulator "$AVD_B" "$PORT_B")

wait_for_device "$SERIAL_A" "$BOOT_TIMEOUT_S"
wait_for_device "$SERIAL_B" "$BOOT_TIMEOUT_S"

# ── Install APK on both ──────────────────────────────────────

echo ""
echo "→ Installing APK on both devices ..."
adb -s "$SERIAL_A" install -r "$APK_PATH"
adb -s "$SERIAL_B" install -r "$APK_PATH"

# ── Run scenario ─────────────────────────────────────────────

echo ""
echo "→ Running scenario: $SCENARIO"
export DEVICE_A_SERIAL="$SERIAL_A"
export DEVICE_B_SERIAL="$SERIAL_B"
export DEVICE_A_AVD="$AVD_A"
export DEVICE_B_AVD="$AVD_B"
export APK_PATH

# Scenarios can be either a Maestro YAML or a shell script.
# Shell scripts are useful for mixed adb+Maestro orchestration;
# pure Maestro YAMLs run on a single device at a time and need
# subflows keyed by `${DEVICE_*_SERIAL}` to drive the other side.
case "$SCENARIO_FILE" in
    *.sh)    bash "$SCENARIO_FILE" ;;
    *.yaml)  maestro test --platform android --device "$SERIAL_A" "$SCENARIO_FILE" ;;
    *)       echo "orchestrator: unsupported scenario extension for $SCENARIO_FILE"; exit 2 ;;
esac

echo ""
echo "✓ Scenario '$SCENARIO' completed"
