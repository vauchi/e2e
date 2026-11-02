#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# A pinned set runs one Core (vauchi/private#1032): every Core crate a
# manifest takes from git must name the set's tag. Before the set, e2e's
# library sat on one Core tag while the CLI and relay it built from their
# mains ran another, and a green run proved a mix no release contained.
#
# Usage: ci/check-core-pin.sh <Cargo.toml> <tag>

set -eu

manifest=${1:?usage: check-core-pin.sh <Cargo.toml> <tag>}
tag=${2:?usage: check-core-pin.sh <Cargo.toml> <tag>}

pins=$(grep -E '^vauchi-(core|app|protocol) *= *\{.*git *=' "$manifest" || true)
if [ -z "$pins" ]; then
    echo "FAIL: $manifest takes no Core crate from git; nothing names a tag" >&2
    exit 1
fi

wrong=$(printf '%s\n' "$pins" | grep -v "tag = \"$tag\"" || true)
if [ -n "$wrong" ]; then
    echo "FAIL: $manifest pins Core crates at another tag than the set's $tag:" >&2
    printf '  %s\n' "$wrong" >&2
    exit 1
fi
echo "OK: $manifest pins every Core crate at $tag"
