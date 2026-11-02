#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The scheduled pins bot (vauchi/private#1032 decision D7, private#1034):
# runs ci/update-pins.sh and, when a pin moved, updates the lockfile,
# commits as the token's bot user on chore/update-pins and opens one MR.
# The MR's pipeline is the gate: the pinned set's lanes run against the
# moved components before anything merges. A branch already under review
# is left alone; it moves again after it merges.
#
# Usage: ci/pins-bot.sh   (from the e2e repo root)
# Env:   PROJECT_ACCESS_TOKEN  group token: git push and MR API
#        CI_API_V4_URL         GitLab API base
#        PINS_PUSH_URL         push target (default the e2e project)

set -eu

branch=chore/update-pins
api=${CI_API_V4_URL:-https://gitlab.com/api/v4}
project=vauchi%2Fe2e
push_url=${PINS_PUSH_URL:-https://vauchi-gitlab-runner:${PROJECT_ACCESS_TOKEN:?}@gitlab.com/vauchi/e2e.git}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Status first, body second: an error body must never be parsed as data.
api_call() { # method url [data]
    if [ -n "${3:-}" ]; then
        curl -sS -o "$work/response.json" -w '%{http_code}' --request "$1" \
            --header "PRIVATE-TOKEN: ${PROJECT_ACCESS_TOKEN:?}" \
            --header "Content-Type: application/json" --data "$3" "$2" || echo 000
    else
        curl -sS -o "$work/response.json" -w '%{http_code}' --request "$1" \
            --header "PRIVATE-TOKEN: ${PROJECT_ACCESS_TOKEN:?}" "$2" || echo 000
    fi
}
field() {
    python3 -c 'import json, sys
data = json.load(open(sys.argv[1]))
if isinstance(data, list):
    data = data[0] if data else {}
print(data.get(sys.argv[2]) or "")' "$work/response.json" "$1"
}

if git ls-remote --exit-code --heads "$push_url" "$branch" > /dev/null 2>&1; then
    status=$(api_call GET "$api/projects/$project/merge_requests?state=opened&source_branch=$branch")
    url=""
    [ "$status" != 200 ] || url=$(field web_url)
    if [ -n "$url" ]; then
        echo "pins: already under review: $url"
        exit 0
    fi
    echo "pins: ERROR: $branch exists without an open merge request (HTTP $status)" >&2
    exit 1
fi

rc=0
sh ci/update-pins.sh > "$work/moves" || rc=$?
cat "$work/moves"
if [ "$rc" = 2 ]; then
    echo "pins: nothing to move"
    exit 0
fi
[ "$rc" = 0 ] || exit "$rc"

# Only the relay is a Cargo dependency of e2e; the binaries are built from
# the pins, not locked here.
cargo update -p vauchi-relay

status=$(api_call GET "$api/user")
[ "$status" = 200 ] || {
    echo "pins: ERROR: cannot resolve the bot identity (HTTP $status)" >&2
    exit 1
}
name=$(field name)
email=$(field commit_email)
[ -n "$email" ] || email=$(field email)

git checkout -q -b "$branch"
git -c user.name="$name" -c user.email="$email" commit -q -a \
    -m "chore: the pinned set follows green component mains" \
    -m "Opened by e2e's pins bot (ci/pins-bot.sh, vauchi/private#1034):" \
    -m "$(cat "$work/moves")"
git push -q "$push_url" "HEAD:refs/heads/$branch"

body=$(python3 -c 'import json, sys
print(json.dumps({
    "source_branch": sys.argv[1],
    "target_branch": "main",
    "title": "chore: the pinned set follows green component mains",
    "description": "Opened by the pins bot (ci/pins-bot.sh, vauchi/private#1034).\n\n"
        + open(sys.argv[2]).read()
        + "\nMerge once the pipeline is green: it runs the pinned set against the moved components.",
    "remove_source_branch": True,
}))' "$branch" "$work/moves")
status=$(api_call POST "$api/projects/$project/merge_requests" "$body")
if [ "$status" != 201 ]; then
    echo "pins: ERROR: pushed $branch but opening the MR failed (HTTP $status)" >&2
    exit 1
fi
echo "pins: opened $(field web_url)"
