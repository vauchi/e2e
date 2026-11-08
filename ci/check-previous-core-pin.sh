#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The support window (vauchi/private#1111, maintainer 2026-11-07): Core
# 0.N interoperates with the newest 0.(N-1).x release. The interop lane's
# previous CLI must therefore run one Core tag of the set's minor or the
# minor below it; anything older tests a pair no window promises, anything
# newer is not a previous release.
#
# Usage: ci/check-previous-core-pin.sh <Cargo.toml> <set-tag>

set -eu

manifest=${1:?usage: check-previous-core-pin.sh <Cargo.toml> <set-tag>}
set_tag=${2:?usage: check-previous-core-pin.sh <Cargo.toml> <set-tag>}

tags=$(grep -E '^vauchi-(core|app|protocol) *= *\{.*git *=' "$manifest" |
    sed -n 's/.*tag = "\(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' | sort -u)
if [ -z "$tags" ]; then
    echo "FAIL: $manifest takes no Core crate from git at a release tag" >&2
    exit 1
fi
if [ "$(printf '%s\n' "$tags" | wc -l | tr -d ' ')" -ne 1 ]; then
    echo "FAIL: $manifest pins Core at more than one tag:" >&2
    printf '  %s\n' "$tags" >&2
    exit 1
fi

major_minor() { # vX.Y.Z -> "X Y"
    printf '%s\n' "$1" | sed 's/^v\([0-9]*\)\.\([0-9]*\)\..*/\1 \2/'
}
# shellcheck disable=SC2046 # word-splitting into two numbers is the point
set -- $(major_minor "$tags") $(major_minor "$set_tag")
previous_major=$1 previous_minor=$2 set_major=$3 set_minor=$4

if [ "$previous_major" -eq "$set_major" ] &&
    { [ "$previous_minor" -eq "$set_minor" ] || [ "$previous_minor" -eq $((set_minor - 1)) ]; }; then
    echo "OK: $manifest runs Core $tags, within the window of $set_tag"
    exit 0
fi
echo "FAIL: $manifest runs Core $tags; the window of $set_tag is its minor or the one below" >&2
exit 1
