#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the relay, the OHTTP anchor CLI, the OHTTP relay and the e2e CLI
# at the commits ci/pins.env names (vauchi/private#1032, decision D5), or
# at each repository's main with E2E_PIN_MODE=heads (the nightly heads
# lane). A cached binary is reused only when its commit marker and its
# build profile match; the profiles select the e2e test clock and fault
# routes these binaries are built with. With E2E_CORE_DIR, the relay and
# the CLI are built against that Core checkout instead of their pinned
# tag, and nothing is cached: a Core merge request testing its own commit
# (decision D8).
#
# Usage: ci/build-pinned-binaries.sh <bin-dir>
# Env:   E2E_PIN_MODE    pinned (default) or heads
#        E2E_CORE_DIR    a Core checkout to build the relay and CLI against
#        CI_JOB_TOKEN    clones from gitlab.com/vauchi in CI

set -eu

bin_dir=${1:?usage: build-pinned-binaries.sh <bin-dir>}
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=pins.env
. "$here/pins.env"

RELAY_BUILD_PROFILE=e2e-test-clock-v1
CLI_BUILD_PROFILE=e2e-test-clock-v3
OHTTP_BUILD_PROFILE=e2e-faults-v1
# The OHTTP relay's fault routes are compiled in only under e2e-faults; a
# binary without this literal 404s them, whatever its marker says.
OHTTP_FAULT_ROUTE='__e2e/reorder-next-forward'

if [ -n "${CI_JOB_TOKEN:-}" ]; then
    gitlab="https://gitlab-ci-token:${CI_JOB_TOKEN}@gitlab.com/vauchi"
else
    gitlab="https://gitlab.com/vauchi"
fi

mode=${E2E_PIN_MODE:-pinned}
case "$mode" in
    pinned) ;;
    heads)
        head_of() { git ls-remote --heads "$gitlab/$1.git" main | cut -f1; }
        RELAY_SHA=$(head_of relay)
        OHTTP_RELAY_SHA=$(head_of ohttp-relay)
        CLI_SHA=$(head_of cli)
        for sha in "$RELAY_SHA" "$OHTTP_RELAY_SHA" "$CLI_SHA"; do
            [ -n "$sha" ] || { echo "FAIL: could not resolve a main head" >&2; exit 1; }
        done
        ;;
    *)
        echo "FAIL: E2E_PIN_MODE must be pinned or heads, not '$mode'" >&2
        exit 2
        ;;
esac

mkdir -p "$bin_dir"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fetch_at() { # repo commit dest
    git init -q "$3"
    git -C "$3" fetch --depth=1 "$gitlab/$1.git" "$2"
    git -C "$3" checkout -q FETCH_HEAD
}

# The component runs the set's Core: its pinned tag, checked, or the
# Core checkout E2E_CORE_DIR names. Heads run as they are.
use_core() { # dest
    if [ -n "${E2E_CORE_DIR:-}" ]; then
        sh "$here/point-core-at.sh" "$E2E_CORE_DIR" "$1/Cargo.toml"
    elif [ "$mode" = pinned ]; then
        sh "$here/check-core-pin.sh" "$1/Cargo.toml" "$CORE_TAG"
    fi
}

cached() { # name commit profile binary...
    [ -z "${E2E_CORE_DIR:-}" ] || return 1
    c_name=$1
    c_commit=$2
    c_profile=$3
    shift 3
    [ -f "$bin_dir/$c_name.sha" ] && [ "$(cat "$bin_dir/$c_name.sha")" = "$c_commit" ] || return 1
    [ -f "$bin_dir/$c_name.build-profile" ] && [ "$(cat "$bin_dir/$c_name.build-profile")" = "$c_profile" ] || return 1
    for c_bin in "$@"; do
        [ -x "$bin_dir/$c_bin" ] || return 1
    done
}

mark() { # name commit profile
    echo "$2" > "$bin_dir/$1.sha"
    echo "$3" > "$bin_dir/$1.build-profile"
}

# The relay has no --version: started bare it refuses to serve without
# TLS and exits 1, so the markers alone decide.
if cached relay "$RELAY_SHA" "$RELAY_BUILD_PROFILE" vauchi-relay vauchi-ohttp-anchor; then
    echo "relay cached at $RELAY_SHA"
else
    echo "Building relay at $RELAY_SHA..."
    fetch_at relay "$RELAY_SHA" "$work/relay"
    use_core "$work/relay"
    cargo build --release --features e2e-test-clock --manifest-path "$work/relay/Cargo.toml"
    cp "$work/relay/target/release/vauchi-relay" "$work/relay/target/release/vauchi-ohttp-anchor" "$bin_dir/"
    mark relay "$RELAY_SHA" "$RELAY_BUILD_PROFILE"
fi

if cached cli "$CLI_SHA" "$CLI_BUILD_PROFILE" vauchi &&
    "$bin_dir/vauchi" --version > /dev/null 2>&1; then
    echo "CLI cached at $CLI_SHA"
else
    echo "Building the e2e CLI at $CLI_SHA..."
    fetch_at cli "$CLI_SHA" "$work/cli"
    use_core "$work/cli"
    cargo build --features e2e-test-clock --manifest-path "$work/cli/Cargo.toml"
    cp "$work/cli/target/debug/vauchi" "$bin_dir/"
    mark cli "$CLI_SHA" "$CLI_BUILD_PROFILE"
fi

if cached ohttp "$OHTTP_RELAY_SHA" "$OHTTP_BUILD_PROFILE" vauchi-ohttp-relay &&
    grep -qa "$OHTTP_FAULT_ROUTE" "$bin_dir/vauchi-ohttp-relay"; then
    echo "OHTTP relay cached at $OHTTP_RELAY_SHA"
else
    echo "Building the OHTTP relay at $OHTTP_RELAY_SHA..."
    fetch_at ohttp-relay "$OHTTP_RELAY_SHA" "$work/ohttp-relay"
    cargo build --release --features e2e-faults --manifest-path "$work/ohttp-relay/Cargo.toml"
    grep -qa "$OHTTP_FAULT_ROUTE" "$work/ohttp-relay/target/release/vauchi-ohttp-relay" || {
        echo "FAIL: the built OHTTP relay lacks its e2e-faults routes" >&2
        exit 1
    }
    cp "$work/ohttp-relay/target/release/vauchi-ohttp-relay" "$bin_dir/"
    mark ohttp "$OHTTP_RELAY_SHA" "$OHTTP_BUILD_PROFILE"
fi

{
    echo "mode=$mode"
    if [ -n "${E2E_CORE_DIR:-}" ]; then
        echo "core=$E2E_CORE_DIR"
    elif [ "$mode" = heads ]; then
        echo "core=each component's own pin"
    else
        echo "core=$CORE_TAG"
    fi
    echo "relay=$RELAY_SHA"
    echo "ohttp_relay=$OHTTP_RELAY_SHA"
    echo "cli=$CLI_SHA"
} > "$bin_dir/set.txt"
echo "Set under test ($mode):"
sed 's/^/  /' "$bin_dir/set.txt"
