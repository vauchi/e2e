#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
FETCH="$SCRIPT_DIR/../fetch-scripts.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home"
mkdir -p "$HOME"

git init --quiet "$WORK/source"
printf 'first\n' > "$WORK/source/content"
git -C "$WORK/source" add content
git -C "$WORK/source" -c user.name=Fixture -c user.email=fixture@example.test commit --quiet -m first
first=$(git -C "$WORK/source" rev-parse HEAD)
git -C "$WORK/source" tag test-template
printf 'second\n' > "$WORK/source/content"
git -C "$WORK/source" add content
git -C "$WORK/source" -c user.name=Fixture -c user.email=fixture@example.test commit --quiet -m second
second=$(git -C "$WORK/source" rev-parse HEAD)
git config --global "url.file://$WORK/source.insteadOf" https://gitlab.com/vauchi/scripts.git

bash "$FETCH" "$first" "$WORK/checkout"
[ "$(git -C "$WORK/checkout" rev-parse HEAD)" = "$first" ]
[ "$(cat "$WORK/checkout/content")" = first ]

bash "$FETCH" "$second" "$WORK/checkout"
[ "$(git -C "$WORK/checkout" rev-parse HEAD)" = "$second" ]
[ "$(cat "$WORK/checkout/content")" = second ]

bash "$FETCH" test-template "$WORK/checkout"
[ "$(git -C "$WORK/checkout" rev-parse HEAD)" = "$first" ]
[ "$(cat "$WORK/checkout/content")" = first ]

if bash "$FETCH" nonexistent "$WORK/checkout"; then
    echo 'FAIL: a missing ref must fail'
    exit 1
fi
[ "$(git -C "$WORK/checkout" rev-parse HEAD)" = "$first" ]
printf 'ok — fresh SHA checkout, existing checkout update, tag pin, and missing-ref failure\n'
