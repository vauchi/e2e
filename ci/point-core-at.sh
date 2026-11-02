#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Points a manifest's Core crates at a local Core checkout, for one run
# (vauchi/private#1032, decision D8). A Core merge request tests its own
# commit end to end: its job clones e2e and builds the pinned relay and
# CLI with their Core dependency rewritten to the MR's checkout. A
# `[patch]` cannot do this, because the exact `version = "=X"` pins refuse
# a Core of another version. Never committed: the job's clones are thrown
# away.
#
# Usage: ci/point-core-at.sh <core-dir> <Cargo.toml>

set -eu

core_dir=${1:?usage: point-core-at.sh <core-dir> <Cargo.toml>}
manifest=${2:?usage: point-core-at.sh <core-dir> <Cargo.toml>}
[ -d "$core_dir/vauchi-core" ] || {
    echo "FAIL: $core_dir is not a Core checkout (no vauchi-core/)" >&2
    exit 1
}

tmp="$manifest.point-core-at"
sed -E "s#^(vauchi-(core|app|protocol)) *= *\\{ *git = \"https://gitlab\\.com/vauchi/core\\.git\", *tag = \"[^\"]*\", *version = \"[^\"]*\"#\\1 = { path = \"$core_dir/\\1\"#" \
    "$manifest" > "$tmp"
mv "$tmp" "$manifest"

left=$(grep -E '^vauchi-(core|app|protocol) *=.*gitlab\.com/vauchi/core\.git' "$manifest" || true)
if [ -n "$left" ]; then
    echo "FAIL: $manifest still takes Core from git; its line has another shape:" >&2
    printf '  %s\n' "$left" >&2
    exit 1
fi
grep -E '^vauchi-(core|app|protocol) *=' "$manifest" | sed "s#^#  $manifest: #" || true
