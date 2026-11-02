#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# The pinned set (vauchi/private#1032, decision D5) holds together:
#
# - ci/pins.env names a Core tag and a full commit per component;
# - e2e's own manifest pins Core at that tag and the relay at that commit,
#   and the lockfile agrees;
# - no lane of .gitlab-ci.yml builds the relay, the OHTTP relay or the CLI
#   from a main head: smoke and integration build through
#   ci/build-pinned-binaries.sh, whose cache follows the pins, and only
#   test:heads, on the schedule, runs the mains;
# - ci/check-core-pin.sh passes a manifest on the set's tag and fails one
#   on another tag or with no Core taken from git (CC-27).
#
# Usage: ci/tests/pins.test.sh   (from the e2e repo root)

set -eu

failures=0
fail() {
    echo "FAIL: $1" >&2
    failures=$((failures + 1))
}
pass() { echo "PASS: $1"; }

# shellcheck source=../pins.env
. ci/pins.env

case "$CORE_TAG" in
    v[0-9]*.[0-9]*.[0-9]*) pass "CORE_TAG $CORE_TAG is a release tag" ;;
    *) fail "CORE_TAG '$CORE_TAG' is not a vX.Y.Z tag" ;;
esac
for pair in "RELAY_SHA=${RELAY_SHA:-}" "OHTTP_RELAY_SHA=${OHTTP_RELAY_SHA:-}" "CLI_SHA=${CLI_SHA:-}"; do
    var=${pair%%=*}
    value=${pair#*=}
    if printf '%s' "$value" | grep -qE '^[0-9a-f]{40}$'; then
        pass "$var is a full commit"
    else
        fail "$var '$value' is not a 40-character commit"
    fi
done

if sh ci/check-core-pin.sh Cargo.toml "$CORE_TAG" > /dev/null; then
    pass "Cargo.toml pins Core at $CORE_TAG"
else
    fail "Cargo.toml must pin every Core crate at CORE_TAG $CORE_TAG"
fi
if grep -qE "^vauchi-relay *=.*rev = \"$RELAY_SHA\"" Cargo.toml; then
    pass "Cargo.toml pins the relay at RELAY_SHA"
else
    fail "Cargo.toml must pin vauchi-relay with rev = \"$RELAY_SHA\""
fi
if grep -A2 '^name = "vauchi-relay"$' Cargo.lock | grep -q "#$RELAY_SHA\"$"; then
    pass "Cargo.lock resolves the relay at RELAY_SHA"
else
    fail "Cargo.lock does not resolve vauchi-relay at $RELAY_SHA; run cargo update -p vauchi-relay"
fi

job_block() {
    awk -v job="$1" '
        $0 == job ":" { in_job = 1; next }
        in_job && /^[^[:space:]#]/ { exit }
        in_job { print }
    ' .gitlab-ci.yml
}

for repo in relay ohttp-relay cli; do
    if grep -B1 -E "gitlab.com/vauchi/$repo\.git" .gitlab-ci.yml | grep -q -- '--branch main'; then
        fail ".gitlab-ci.yml clones $repo from main; build it at its pin"
    else
        pass "no lane clones $repo from main"
    fi
done
if grep -q 'ls-remote --heads' .gitlab-ci.yml; then
    fail ".gitlab-ci.yml resolves a main head; only build-pinned-binaries.sh in heads mode may"
else
    pass "no lane resolves a main head"
fi

for job in test:smoke test:integration; do
    block=$(job_block "$job")
    printf '%s\n' "$block" | grep -q 'sh ci/build-pinned-binaries.sh .smoke-bin' ||
        fail "$job must build through ci/build-pinned-binaries.sh"
    printf '%s\n' "$block" | grep -q 'files: \[ci/pins.env, ci/build-pinned-binaries.sh\]' ||
        fail "$job's cache must be keyed by ci/pins.env and the build script"
done

heads=$(job_block test:heads)
if [ -z "$heads" ]; then
    fail "test:heads is not in .gitlab-ci.yml"
else
    printf '%s\n' "$heads" | grep -q 'E2E_PIN_MODE: heads' || fail "test:heads must set E2E_PIN_MODE: heads"
    printf '%s\n' "$heads" | grep -q 'CI_PIPELINE_SOURCE == "schedule"' || fail "test:heads must run on the schedule"
    if printf '%s\n' "$heads" | grep -q 'merge_request_event\|CI_DEFAULT_BRANCH'; then
        fail "test:heads must run on the schedule only"
    fi
fi

fixtures=$(mktemp -d)
trap 'rm -rf "$fixtures"' EXIT
cat > "$fixtures/on-tag.toml" <<'EOF'
vauchi-core = { git = "https://gitlab.com/vauchi/core.git", tag = "v1.2.3", version = "=1.2.3" }
vauchi-protocol = { git = "https://gitlab.com/vauchi/core.git", tag = "v1.2.3", version = "=1.2.3" }
EOF
cat > "$fixtures/other-tag.toml" <<'EOF'
vauchi-core = { git = "https://gitlab.com/vauchi/core.git", tag = "v1.2.3", version = "=1.2.3" }
vauchi-app = { git = "https://gitlab.com/vauchi/core.git", tag = "v1.2.4", version = "=1.2.4" }
EOF
cat > "$fixtures/no-git.toml" <<'EOF'
vauchi-core = { path = "../core/vauchi-core" }
EOF
if sh ci/check-core-pin.sh "$fixtures/on-tag.toml" v1.2.3 > /dev/null 2>&1; then
    pass "check-core-pin passes a manifest on the set's tag"
else
    fail "check-core-pin must pass a manifest on the set's tag"
fi
if sh ci/check-core-pin.sh "$fixtures/other-tag.toml" v1.2.3 > /dev/null 2>&1; then
    fail "check-core-pin must fail a manifest with a crate on another tag"
else
    pass "check-core-pin fails a crate on another tag"
fi
if sh ci/check-core-pin.sh "$fixtures/no-git.toml" v1.2.3 > /dev/null 2>&1; then
    fail "check-core-pin must fail a manifest that takes no Core from git"
else
    pass "check-core-pin fails a manifest with no Core from git"
fi

# point-core-at.sh (decision D8): a Core MR's job points the pinned
# components at its checkout; the known line shape is rewritten with its
# features kept, any other shape or a non-Core directory fails loudly.
mkdir -p "$fixtures/core/vauchi-core" "$fixtures/not-core"
cat > "$fixtures/to-point.toml" <<'EOF'
vauchi-core = { git = "https://gitlab.com/vauchi/core.git", tag = "v1.2.3", version = "=1.2.3" }
vauchi-app = { git = "https://gitlab.com/vauchi/core.git", tag = "v1.2.3", version = "=1.2.3", features = ["x"] }
EOF
if sh ci/point-core-at.sh "$fixtures/core" "$fixtures/to-point.toml" > /dev/null 2>&1 &&
    grep -qx "vauchi-core = { path = \"$fixtures/core/vauchi-core\" }" "$fixtures/to-point.toml" &&
    grep -qx "vauchi-app = { path = \"$fixtures/core/vauchi-app\", features = \[\"x\"\] }" "$fixtures/to-point.toml"; then
    pass "point-core-at rewrites Core crates to the checkout and keeps their features"
else
    fail "point-core-at must rewrite Core crates to path dependencies, keeping features"
fi
printf 'vauchi-core = { tag = "v1", git = "https://gitlab.com/vauchi/core.git" }\n' > "$fixtures/odd.toml"
if sh ci/point-core-at.sh "$fixtures/core" "$fixtures/odd.toml" > /dev/null 2>&1; then
    fail "point-core-at must fail a Core line it cannot rewrite"
else
    pass "point-core-at fails a Core line of another shape"
fi
if sh ci/point-core-at.sh "$fixtures/not-core" "$fixtures/on-tag.toml" > /dev/null 2>&1; then
    fail "point-core-at must refuse a directory that is not a Core checkout"
else
    pass "point-core-at refuses a directory without vauchi-core/"
fi

if [ "$failures" -gt 0 ]; then
    echo "$failures failure(s)" >&2
    exit 1
fi
echo "OK: the pinned set holds together"
