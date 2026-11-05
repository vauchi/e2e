#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

set -eu
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
fetch="$script_dir/../fetch-features.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM
git init --quiet "$work/source"
printf 'Feature: First revision\n' > "$work/source/link_exchange.feature"
git -C "$work/source" add link_exchange.feature
git -C "$work/source" -c user.name=Fixture -c user.email=fixture@example.test commit --quiet -m first
first=$(git -C "$work/source" rev-parse HEAD)
printf 'Feature: Second revision\n' > "$work/source/link_exchange.feature"
git -C "$work/source" add link_exchange.feature
git -C "$work/source" -c user.name=Fixture -c user.email=fixture@example.test commit --quiet -m second
second=$(git -C "$work/source" rev-parse HEAD)
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0="url.file://$work/source.insteadOf"
export GIT_CONFIG_VALUE_0=https://gitlab.com/vauchi/features.git
sh "$fetch" "$first" "$work/checkout"
[ "$(cat "$work/checkout/link_exchange.feature")" = 'Feature: First revision' ]
sh "$fetch" "$second" "$work/checkout"
[ "$(cat "$work/checkout/link_exchange.feature")" = 'Feature: Second revision' ]
if sh "$fetch" nonexistent "$work/checkout"; then
    echo 'FAIL: a missing feature ref must stop the job' >&2
    exit 1
fi
[ "$(git -C "$work/checkout" rev-parse HEAD)" = "$second" ]
echo 'PASS: canonical feature fetch updates its revision and fails closed on a missing ref'
