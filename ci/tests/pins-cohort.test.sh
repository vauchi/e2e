#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# ci/pins-cohort.sh services an open Core-bump MR (private#1210): with no
# such MR it does nothing; when the cohort's update-pins moves a pin, the
# moved pin is committed by the bot user on the EXISTING chore/bump-core-*
# branch (main stays untouched) and no second MR is opened; a cohort that
# cannot move yet leaves the branch alone and the job green. A local bare
# repo stands in for e2e, fakes for update-pins.sh, cargo and the API.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() {
    echo "FAIL: $*" >&2
    exit 1
}
mkdir -p "$t/bin"
git_q() { git -c init.defaultBranch=main -c user.name=t -c user.email=t@example.invalid "$@" >/dev/null 2>&1; }

setup() { # update-pins exit code
    rm -rf "$t/src" "$t/remote.git" "$t/work" "$t/log"
    mkdir -p "$t/src/ci"
    cp "$here/ci/pins-cohort.sh" "$t/src/ci/"
    printf 'CORE_TAG=v1\nRELAY_SHA=old\n' > "$t/src/ci/pins.env"
    printf '#!/bin/sh\n[ "%s" = 2 ] && exit 2\nsed s/RELAY_SHA=old/RELAY_SHA=new/ ci/pins.env > ci/pins.env.new && cat ci/pins.env.new > ci/pins.env\necho "relay: moved old -> new"\n' "$1" \
        > "$t/src/ci/update-pins.sh"
    echo lock > "$t/src/Cargo.lock"
    git_q init -q "$t/src"
    git_q -C "$t/src" add -A
    git_q -C "$t/src" commit -q -m init
    # The open Core-bump MR's branch: CORE_TAG moved, RELAY_SHA did not.
    git_q -C "$t/src" checkout -q -b chore/bump-core-0.106.1
    printf 'CORE_TAG=v2\nRELAY_SHA=old\n' > "$t/src/ci/pins.env"
    git_q -C "$t/src" commit -qam 'chore: pin Core v2'
    git_q -C "$t/src" checkout -q main
    git_q clone -q --bare "$t/src" "$t/remote.git"
    git_q clone -q "$t/remote.git" "$t/work"
}

cat > "$t/bin/cargo" <<'EOF'
#!/bin/sh
echo "cargo $*" >> "$FAKE_LOG"
echo "lock updated" > Cargo.lock
EOF
cat > "$t/bin/curl" <<'EOF'
#!/bin/sh
out=""; method=GET; url=""; data=""
while [ "$#" -gt 0 ]; do
    case "$1" in -o) out=$2; shift ;; --request) method=$2; shift ;; --data) data=$2; shift ;;
        -w|--header) shift ;; -*) ;; *) url=$1 ;; esac
    shift
done
echo "$method $url $data" >> "$FAKE_LOG"
case "$method $url" in
    "GET "*/user) body='{"name":"Pins Bot","email":"bot@example.invalid"}'; status=200 ;;
    "GET "*/merge_requests*) body="${FAKE_OPEN_MR:-[]}"; status=200 ;;
    "POST "*/merge_requests) body='{"web_url":"https://gitlab.example/mr/1"}'; status=201 ;;
    *) body='{}'; status=404 ;;
esac
printf '%s' "$body" > "$out"
printf '%s' "$status"
EOF
chmod +x "$t/bin/cargo" "$t/bin/curl"

run() {
    rc=0
    (cd "$t/work" && PATH="$t/bin:$PATH" FAKE_LOG="$t/log" PINS_PUSH_URL="$t/remote.git" \
        CI_API_V4_URL=https://gitlab.example/api/v4 PROJECT_ACCESS_TOKEN=token sh ci/pins-cohort.sh) \
        > "$t/out" 2>&1 || rc=$?
    touch "$t/log"
}

# No open Core-bump MR: nothing happens, even when pins could move.
setup 0
FAKE_OPEN_MR='[]'
export FAKE_OPEN_MR
run
unset FAKE_OPEN_MR
[ "$rc" = 0 ] || fail "no open bump MR must exit 0, got $rc: $(cat "$t/out")"
git -C "$t/remote.git" rev-parse -q --verify refs/heads/chore/bump-core-0.106.1 >/dev/null || fail "the fixture branch must exist"
[ "$(git -C "$t/remote.git" show chore/bump-core-0.106.1:ci/pins.env | grep RELAY_SHA)" = RELAY_SHA=old ] || fail "no open bump MR must move nothing"
grep -q cargo "$t/log" && fail "no open bump MR must not touch the lockfile"

# An open bump MR whose cohort moved: the existing branch is advanced,
# main keeps the old pin, and no second MR is opened.
setup 0
# The fake curl prints this JSON as the open-MR lookup's body.
# shellcheck disable=SC2089,SC2090
FAKE_OPEN_MR='[{"source_branch":"chore/bump-core-0.106.1","web_url":"https://gitlab.example/mr/7"}]'
# shellcheck disable=SC2090
export FAKE_OPEN_MR
run
unset FAKE_OPEN_MR
cat "$t/out"
[ "$rc" = 0 ] || fail "a moved cohort pin must exit 0, got $rc: $(cat "$t/out")"
[ "$(git -C "$t/remote.git" show chore/bump-core-0.106.1:ci/pins.env | grep RELAY_SHA)" = RELAY_SHA=new ] || fail "the bump branch must carry the moved pin"
[ "$(git -C "$t/remote.git" show main:ci/pins.env | grep RELAY_SHA)" = RELAY_SHA=old ] || fail "main must keep its own pin"
[ "$(git -C "$t/remote.git" show chore/bump-core-0.106.1:Cargo.lock)" = "lock updated" ] || fail "the bump branch lockfile must be updated"
[ "$(git -C "$t/remote.git" log -1 --format=%ae chore/bump-core-0.106.1)" = bot@example.invalid ] || fail "the cohort commit must be the bot user's"
grep -q '^POST .*merge_requests' "$t/log" && fail "the cohort must not open a second MR"
grep -q '^CORE_TAG=v2' "$t/out" && fail "unrelated output"

# An open bump MR whose components are not on the new Core yet: green, no push.
setup 2
# shellcheck disable=SC2089,SC2090
FAKE_OPEN_MR='[{"source_branch":"chore/bump-core-0.106.1","web_url":"https://gitlab.example/mr/7"}]'
# shellcheck disable=SC2090
export FAKE_OPEN_MR
before=$(git -C "$t/remote.git" rev-parse chore/bump-core-0.106.1)
run
unset FAKE_OPEN_MR
[ "$rc" = 0 ] || fail "a waiting cohort must exit 0, got $rc: $(cat "$t/out")"
[ "$(git -C "$t/remote.git" rev-parse chore/bump-core-0.106.1)" = "$before" ] || fail "a waiting cohort must not push"
grep -q '^POST .*merge_requests' "$t/log" && fail "a waiting cohort must open no MR"
echo "OK: the pins cohort advances only the open Core-bump branch, only when a pin moved"
