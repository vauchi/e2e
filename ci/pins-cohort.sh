#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The cohort half of the pins bot (vauchi/private#1210): a Core release's
# chore/bump-core-* MR moves CORE_TAG and Cargo.toml, but RELAY_SHA and
# CLI_SHA can only follow once the relay and CLI mains pin that new tag.
# Running ci/update-pins.sh on main never gets there — main's CORE_TAG is
# still the old release, so the bump MR could never turn green on its own.
# This script runs the same update-pins.sh inside a worktree of the open
# bump MR's branch, where CORE_TAG is already the new release, and pushes
# the moved pins back to that branch. The existing MR's pipeline reruns;
# no second MR is opened. A cohort whose component mains are not on the
# new Core yet waits for the next scheduled run.
#
# Usage: ci/pins-cohort.sh   (from the e2e repo root, after ci/pins-bot.sh)
# Env:   PROJECT_ACCESS_TOKEN  group token: git push and MR API
#        CI_API_V4_URL         GitLab API base
#        PINS_PUSH_URL         push target (default the e2e project)

set -eu

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

status=$(api_call GET "$api/projects/$project/merge_requests?state=opened&per_page=100")
[ "$status" = 200 ] || {
    echo "cohort: ERROR: cannot list open merge requests (HTTP $status)" >&2
    exit 1
}
branch=$(python3 -c 'import json
for mr in json.load(open("'"$work"'/response.json")):
    source = mr.get("source_branch") or ""
    if source.startswith("chore/bump-core-"):
        print(source)
        break')
if [ -z "$branch" ]; then
    echo "cohort: no open Core bump MR"
    exit 0
fi

git fetch -q "$push_url" "$branch"
git worktree add -q --detach "$work/cohort" FETCH_HEAD

rc=0
(cd "$work/cohort" && sh ci/update-pins.sh) > "$work/moves" || rc=$?
cat "$work/moves"
if [ "$rc" = 2 ]; then
    echo "cohort: $branch waits for component mains on its Core tag"
    exit 0
fi
[ "$rc" = 0 ] || exit "$rc"

# Only the relay is a Cargo dependency of e2e; the binaries are built from
# the pins, not locked here.
(cd "$work/cohort" && cargo update -p vauchi-relay)

status=$(api_call GET "$api/user")
[ "$status" = 200 ] || {
    echo "cohort: ERROR: cannot resolve the bot identity (HTTP $status)" >&2
    exit 1
}
name=$(field name)
email=$(field commit_email)
[ -n "$email" ] || email=$(field email)

git -C "$work/cohort" -c user.name="$name" -c user.email="$email" commit -q -a \
    -m "chore: the Core-bump cohort follows green component mains" \
    -m "Advanced by e2e's pins bot (ci/pins-cohort.sh, vauchi/private#1210):" \
    -m "$(cat "$work/moves")"
git -C "$work/cohort" push -q "$push_url" "HEAD:refs/heads/$branch"
echo "cohort: advanced $branch; its open MR's pipeline reruns"
