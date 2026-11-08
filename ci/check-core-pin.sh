#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A pinned set runs one Core (vauchi/private#1032): every Core crate a
# manifest takes from git must name the set's tag. Before the set, e2e's
# library sat on one Core tag while the CLI and relay it built from their
# mains ran another, and a green run proved a mix no release contained.
#
# Usage: ci/check-core-pin.sh <Cargo.toml> <tag>
# Exit 0 for matching pins, 1 for violations, >1 for operational errors.

set -eu

manifest=${1:?usage: check-core-pin.sh <Cargo.toml> <tag>}
tag=${2:?usage: check-core-pin.sh <Cargo.toml> <tag>}

if pins=$(grep -E '^vauchi-(core|app|protocol) *= *\{.*git *=' "$manifest"); then
    :
else
    search_status=$?
    if [ "$search_status" -ne 1 ]; then
        echo "ERROR: could not read Core pins from $manifest" >&2
        exit "$search_status"
    fi
fi
if [ -z "$pins" ]; then
    echo "FAIL: $manifest takes no Core crate from git; nothing names a tag" >&2
    exit 1
fi

if wrong=$(printf '%s\n' "$pins" | grep -F -v -- "tag = \"$tag\""); then
    :
else
    search_status=$?
    if [ "$search_status" -ne 1 ]; then
        echo "ERROR: could not compare Core pins with $tag" >&2
        exit "$search_status"
    fi
fi
if [ -n "$wrong" ]; then
    echo "FAIL: $manifest pins Core crates at another tag than the set's $tag:" >&2
    printf '  %s\n' "$wrong" >&2
    exit 1
fi
echo "OK: $manifest pins every Core crate at $tag"
