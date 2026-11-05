#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

set -eu

config=${1:-.gitlab-ci.yml}
for job in security:audit security:deny; do
    block=$(awk -v job="$job" '
        $0 == job ":" { in_job = 1; next }
        in_job && /^[^[:space:]#]/ { exit }
        in_job { print }
    ' "$config")
    if ! printf '%s\n' "$block" | grep -Eq '^  needs: \[\][[:space:]]*$'; then
        echo "FAIL: $job must start independently of builds and lint jobs" >&2
        exit 1
    fi
    echo "PASS: $job can start before Clippy and cold builds"
done
