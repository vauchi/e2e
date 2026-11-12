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
# Usage: ci/check-core-pin.sh <Cargo.toml> <tag> [Cargo.lock]
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

if [ "$#" -ge 3 ]; then
    lockfile=$3
    if [ ! -f "$lockfile" ]; then
        echo "ERROR: could not read Core lockfile $lockfile" >&2
        exit 2
    fi
    if wrong_lock=$(awk -v expected="git+https://gitlab.com/vauchi/core.git?tag=$tag#" '
        function check_package() {
            if (core_package && index(source, expected) == 0)
                print name ": " source
        }
        /^\[\[package\]\]$/ {
            check_package()
            core_package = 0
            name = ""
            source = ""
            next
        }
        /^name = "vauchi-(core|app|protocol)"$/ {
            core_package = 1
            name = $0
            next
        }
        core_package && /^source = / { source = $0 }
        END { check_package() }
    ' "$lockfile"); then
        :
    else
        search_status=$?
        echo "ERROR: could not compare Core lockfile with $tag" >&2
        exit "$search_status"
    fi
    if [ -n "$wrong_lock" ]; then
        echo "FAIL: $lockfile resolves Core crates at another tag than the set's $tag:" >&2
        printf '  %s\n' "$wrong_lock" >&2
        exit 1
    fi
fi
echo "OK: Core pins match $tag"
