#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# ci/pins-bot.sh, the scheduled job around ci/update-pins.sh
# (private#1034): nothing to move pushes nothing; a moved pin is committed
# by the token's bot user on chore/update-pins and gets exactly one MR; a
# branch already under review is left alone. A local bare repo stands in
# for e2e, fakes for update-pins.sh, cargo and the GitLab API.
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
    cp "$here/ci/pins-bot.sh" "$t/src/ci/"
    printf 'CORE_TAG=v1\nRELAY_SHA=old\n' > "$t/src/ci/pins.env"
    printf '#!/bin/sh\n[ "%s" = 2 ] && exit 2\nsed -i s/RELAY_SHA=old/RELAY_SHA=new/ ci/pins.env\necho "relay: moved old -> new"\n' "$1" \
        > "$t/src/ci/update-pins.sh"
    echo lock > "$t/src/Cargo.lock"
    git_q init -q "$t/src"
    git_q -C "$t/src" add -A
    git_q -C "$t/src" commit -q -m init
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
        CI_API_V4_URL=https://gitlab.example/api/v4 PROJECT_ACCESS_TOKEN=token sh ci/pins-bot.sh) \
        > "$t/out" 2>&1 || rc=$?
    touch "$t/log"
}

setup 2
run
[ "$rc" = 0 ] || fail "nothing to move must exit 0, got $rc"
git -C "$t/remote.git" rev-parse -q --verify refs/heads/chore/update-pins >/dev/null && fail "nothing to move must push nothing"
grep -q merge_requests "$t/log" && fail "nothing to move must open no MR"

setup 0
run
cat "$t/out"
[ "$rc" = 0 ] || fail "a moved pin must exit 0, got $rc"
git -C "$t/remote.git" rev-parse -q --verify refs/heads/chore/update-pins >/dev/null || fail "a moved pin must push chore/update-pins"
[ "$(git -C "$t/remote.git" show chore/update-pins:ci/pins.env | grep RELAY_SHA)" = RELAY_SHA=new ] || fail "the pushed branch must carry the moved pin"
[ "$(git -C "$t/remote.git" show chore/update-pins:Cargo.lock)" = "lock updated" ] || fail "the lockfile must be updated with the pin"
[ "$(git -C "$t/remote.git" log -1 --format=%ae chore/update-pins)" = bot@example.invalid ] || fail "the commit must be the bot user's"
[ "$(grep -c '^POST .*merge_requests' "$t/log")" -eq 1 ] || fail "a moved pin must open exactly one MR"

setup 0
git_q -C "$t/src" push -q "$t/remote.git" HEAD:refs/heads/chore/update-pins
# The fake curl prints this JSON as the open-MR lookup's body.
# shellcheck disable=SC2089,SC2090
FAKE_OPEN_MR='[{"web_url":"https://gitlab.example/mr/9"}]'
# shellcheck disable=SC2090
export FAKE_OPEN_MR
run
unset FAKE_OPEN_MR
[ "$rc" = 0 ] || fail "a branch under review must leave the job green, got $rc"
grep -q '^POST .*merge_requests' "$t/log" && fail "a branch under review must get no second MR"
grep -q "mr/9" "$t/out" || fail "the job must point at the open MR"
echo "OK: the pins bot commits, pushes and opens one MR only when a pin moved"
