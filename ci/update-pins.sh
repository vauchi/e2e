#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The pins bot (vauchi/private#1032 decision D7, private#1034): moves each
# component of the pinned set in ci/pins.env to the head of its main, but
# only when that head's main pipeline passed and, for the relay and the
# CLI, the head pins the set's Core tag. A head that is not there yet waits
# for a later run. RELAY_SHA also moves e2e's own vauchi-relay rev in
# Cargo.toml; the job then updates the lockfile and opens the MR.
#
# Usage: ci/update-pins.sh   (from the e2e repo root)
# Env:   PINS_GIT_BASE         where the components live (default gitlab.com/vauchi)
#        PINS_API              GitLab API base (default CI_API_V4_URL or gitlab.com)
#        PROJECT_ACCESS_TOKEN  read access to the pipelines API, when set
# Exit:  0 a pin moved; 2 nothing to move.

set -eu

here=$(cd "$(dirname "$0")" && pwd)
pins="$here/pins.env"
manifest="$here/../Cargo.toml"
base=${PINS_GIT_BASE:-https://gitlab.com/vauchi}
api=${PINS_API:-${CI_API_V4_URL:-https://gitlab.com/api/v4}}
# shellcheck source=pins.env
. "$pins"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pipeline_status() { # repo sha
    url="$api/projects/vauchi%2F$1/pipelines?ref=main&sha=$2&per_page=1"
    if [ -n "${PROJECT_ACCESS_TOKEN:-}" ]; then
        status=$(curl -sS -o "$work/pipelines.json" -w '%{http_code}' \
            --header "PRIVATE-TOKEN: $PROJECT_ACCESS_TOKEN" "$url") || status=000
    else
        status=$(curl -sS -o "$work/pipelines.json" -w '%{http_code}' "$url") || status=000
    fi
    if [ "$status" != 200 ]; then
        echo "http-$status"
        return 0
    fi
    python3 -c 'import json, sys
runs = json.load(open(sys.argv[1]))
print(runs[0]["status"] if runs else "none")' "$work/pipelines.json"
}

set_pin() { # VAR sha
    sed "s/^$1=.*/$1=$2/" "$pins" > "$work/pins.env"
    cat "$work/pins.env" > "$pins"
}

moved=0
for entry in relay:RELAY_SHA:core cli:CLI_SHA:core ohttp-relay:OHTTP_RELAY_SHA:none; do
    repo=${entry%%:*}
    rest=${entry#*:}
    var=${rest%%:*}
    pinned_core=${rest#*:}
    case "$var" in
        RELAY_SHA) current=$RELAY_SHA ;;
        CLI_SHA) current=$CLI_SHA ;;
        *) current=$OHTTP_RELAY_SHA ;;
    esac
    head=$(git ls-remote "$base/$repo.git" refs/heads/main | cut -f1)
    if [ -z "$head" ]; then
        echo "$repo: waits: could not read the head of main"
        continue
    fi
    if [ "$head" = "$current" ]; then
        echo "$repo: current at $head"
        continue
    fi
    run=$(pipeline_status "$repo" "$head")
    if [ "$run" != success ]; then
        echo "$repo: waits: main's head $head has pipeline status $run"
        continue
    fi
    if [ "$pinned_core" = core ]; then
        git init -q "$work/$repo"
        git -C "$work/$repo" fetch -q --depth=1 "$base/$repo.git" "$head"
        git -C "$work/$repo" show FETCH_HEAD:Cargo.toml > "$work/$repo.toml"
        if ! sh "$here/check-core-pin.sh" "$work/$repo.toml" "$CORE_TAG" > /dev/null 2>&1; then
            echo "$repo: waits: main's head $head pins another Core tag than $CORE_TAG"
            continue
        fi
    fi
    set_pin "$var" "$head"
    if [ "$var" = RELAY_SHA ]; then
        sed "s/^\(vauchi-relay = .*rev = \"\)[0-9a-f]*\"/\1$head\"/" "$manifest" > "$work/Cargo.toml"
        cat "$work/Cargo.toml" > "$manifest"
    fi
    echo "$repo: moved $current -> $head"
    moved=$((moved + 1))
done

[ "$moved" -gt 0 ] || exit 2
