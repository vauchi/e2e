#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The TUI journeys: e2e drives `vauchi-tui`, a binary that runs the app
# engine, against real relays. It is the one lane that failed when the sync
# call was removed from the engine (vauchi/private#1297, ADR-082 item 3).
#
# The TUI is built in a job-private directory. On a shell runner the tui
# project's own job workspace is `../tui` of this checkout, so a sibling
# clone there deleted a concurrent tui job's build tree and vice versa
# (allyson, 2026-09-12). The relays are never taken from a sibling checkout
# for the same reason.
#
# With E2E_CORE_DIR the TUI is built against that Core checkout instead of
# the tag tui pins: Core's merge requests run the engine under review, not
# the last released one.
#
# Usage: ci/tui-journey.sh <job-dir> -E <nextest filter>
# Env:   E2E_CORE_DIR   a Core checkout to build the TUI against
#        E2E_BIN_DIR    where the relay binaries are kept and reused; the
#                       job directory when unset
#        CI_JOB_TOKEN   clones from gitlab.com/vauchi in CI
# Run from the e2e checkout's root.
set -eu

usage="usage: tui-journey.sh <job-dir> -E <nextest filter>"
job_dir=${1:?$usage}
[ "${2:-}" = -E ] || {
    echo "$usage" >&2
    exit 2
}
filter=${3:?$usage}

if [ -n "${CI_JOB_TOKEN:-}" ]; then
    origin="https://gitlab-ci-token:${CI_JOB_TOKEN}@gitlab.com/vauchi"
else
    origin="https://gitlab.com/vauchi"
fi

sh ci/fetch-features.sh "${VAUCHI_FEATURES_REF:-main}" .feature-specs
VAUCHI_FEATURES_DIR="$(pwd)/.feature-specs"
export VAUCHI_FEATURES_DIR

rm -rf "$job_dir"
mkdir -p "$job_dir"

# The TUI stays at its main until its journey moves to its app.
git clone --depth=20 --branch main "$origin/tui.git" "$job_dir/tui"
git -C "$job_dir/tui" log --oneline -1
# tui embeds themes/generated/tokens_ansi.json from a sibling checkout
# (tui/src/theme.rs). themes/generated/ is not committed: the generator
# writes it, and a missing file must fail here, not as a rustc include error.
git clone --depth=1 --branch main "$origin/themes.git" "$job_dir/themes"
python3 "$job_dir/themes/scripts/generate.py"
test -f "$job_dir/themes/generated/tokens_ansi.json"
if [ -n "${E2E_CORE_DIR:-}" ]; then
    sh ci/point-core-at.sh "$E2E_CORE_DIR" "$job_dir/tui/Cargo.toml"
fi
cargo build --release --manifest-path "$job_dir/tui/Cargo.toml"
VAUCHI_TUI_BIN="$job_dir/tui/target/release/vauchi-tui"
export VAUCHI_TUI_BIN
test -x "$VAUCHI_TUI_BIN"

# Both relays at the commits ci/pins.env names (vauchi/private#1032), built
# by the script the smoke job uses, so a bin directory that already holds
# them at those commits is reused: building them cold was most of this
# lane's ten minutes. They stay on their own pins whatever Core the TUI is
# built on, and the CLI is not built: no journey runs it.
bin_dir=${E2E_BIN_DIR:-$job_dir/bin}
(
    unset E2E_CORE_DIR
    E2E_DEVICE_BACKEND=core sh ci/build-pinned-binaries.sh "$bin_dir"
)
E2E_BIN_DIR=$bin_dir
export E2E_BIN_DIR
test -x "$E2E_BIN_DIR/vauchi-relay"
test -x "$E2E_BIN_DIR/vauchi-ohttp-anchor"
test -x "$E2E_BIN_DIR/vauchi-ohttp-relay"

cargo nextest run --profile ci --run-ignored default -j 1 --features tui -E "$filter"
