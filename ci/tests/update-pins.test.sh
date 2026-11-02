#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# ci/update-pins.sh, the pins bot (vauchi/private#1032 decision D7,
# private#1034): each component of the pinned set moves to the head of its
# main only when that head's pipeline passed and, for the relay and the
# CLI, the head pins the set's Core tag. Local bare repos stand in for the
# components; a fake curl answers the pipeline lookups.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() {
    echo "FAIL: $*" >&2
    exit 1
}
git_q() { git -c init.defaultBranch=main -c user.name=t -c user.email=t@example.invalid "$@" >/dev/null 2>&1; }

remote() { # name cargo-toml-or-empty
    git_q init -q "$t/src/$1"
    if [ -n "$2" ]; then printf '%s\n' "$2" > "$t/src/$1/Cargo.toml"; else echo x > "$t/src/$1/README"; fi
    git_q -C "$t/src/$1" add -A
    git_q -C "$t/src/$1" commit -q -m init
    git_q clone -q --bare "$t/src/$1" "$t/remotes/$1.git"
    git -C "$t/remotes/$1.git" rev-parse HEAD
}
core_line() { echo "vauchi-protocol = { git = \"https://gitlab.com/vauchi/core.git\", tag = \"$1\", version = \"=${1#v}\" }"; }

mkdir -p "$t/remotes" "$t/bin" "$t/e2e/ci"
relay_head=$(remote relay "$(core_line v0.97.0)")
cli_head=$(remote cli "$(core_line v0.96.1)")
ohttp_head=$(remote ohttp-relay "")

old=1111111111111111111111111111111111111111
cp "$here/ci/update-pins.sh" "$here/ci/check-core-pin.sh" "$t/e2e/ci/"
cat > "$t/e2e/ci/pins.env" <<EOF
# set
CORE_TAG=v0.97.0
RELAY_SHA=$old
OHTTP_RELAY_SHA=$old
CLI_SHA=$old
EOF
cat > "$t/e2e/Cargo.toml" <<EOF
vauchi-relay = { git = "https://gitlab.com/vauchi/relay.git", rev = "$old" }
EOF

# Pipelines: relay's head passed, ohttp-relay's failed, cli's passed (but
# it pins another Core tag).
cat > "$t/bin/curl" <<EOF
#!/bin/sh
out=""; url=""
while [ "\$#" -gt 0 ]; do
    case "\$1" in -o) out=\$2; shift ;; -w|--header|--request) shift ;; -*) ;; *) url=\$1 ;; esac
    shift
done
case "\$url" in
    *vauchi%2Frelay/pipelines*sha=$relay_head*) body='[{"status":"success"}]' ;;
    *vauchi%2Fcli/pipelines*sha=$cli_head*) body='[{"status":"success"}]' ;;
    *vauchi%2Fohttp-relay/pipelines*sha=$ohttp_head*) body='[{"status":"failed"}]' ;;
    *) body='[]' ;;
esac
printf '%s' "\$body" > "\$out"
printf 200
EOF
chmod +x "$t/bin/curl"

run() {
    rc=0
    (cd "$t/e2e" && PATH="$t/bin:$PATH" PINS_GIT_BASE="$t/remotes" PINS_API=https://gitlab.example/api/v4 \
        sh ci/update-pins.sh) > "$t/out" 2>&1 || rc=$?
}

run
cat "$t/out"
[ "$rc" = 0 ] || fail "a moved pin must exit 0, got $rc"
grep -qx "RELAY_SHA=$relay_head" "$t/e2e/ci/pins.env" || fail "the relay must move to its green head on the set's Core tag"
grep -q "rev = \"$relay_head\"" "$t/e2e/Cargo.toml" || fail "e2e's relay dependency must follow RELAY_SHA"
grep -qx "CLI_SHA=$old" "$t/e2e/ci/pins.env" || fail "a CLI head on another Core tag must wait"
grep -q "cli: waits" "$t/out" || fail "the waiting CLI must be reported"
grep -qx "OHTTP_RELAY_SHA=$old" "$t/e2e/ci/pins.env" || fail "a head whose pipeline failed must wait"
grep -q "ohttp-relay: waits" "$t/out" || fail "the waiting OHTTP relay must be reported"
grep -qx "CORE_TAG=v0.97.0" "$t/e2e/ci/pins.env" || fail "the bot must leave CORE_TAG alone"

run
[ "$rc" = 2 ] || fail "with nothing to move the bot must exit 2, got $rc"
echo "OK: the pins bot moves green heads on the set's Core tag and waits for the rest"
