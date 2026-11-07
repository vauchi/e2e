#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
mkdir -p "$t/ci" "$t/bin" "$t/cache"
cp "$here/ci/build-pinned-binaries.sh" "$here/ci/check-core-pin.sh" "$t/ci/"
sha=1111111111111111111111111111111111111111
cat > "$t/ci/pins.env" <<EOF
CORE_TAG=v2.0.0
RELAY_SHA=$sha
CLI_SHA=$sha
OHTTP_RELAY_SHA=$sha
EOF
for binary in vauchi-relay vauchi-ohttp-anchor vauchi vauchi-ohttp-relay; do
    cat > "$t/cache/$binary" <<'EOF'
#!/bin/sh
# __e2e/reorder-next-forward
exit 0
EOF
    chmod +x "$t/cache/$binary"
done
for component in relay cli ohttp; do
    echo "$sha" > "$t/cache/$component.sha"
done
echo e2e-test-clock-v1 > "$t/cache/relay.build-profile"
echo e2e-test-clock-v3 > "$t/cache/cli.build-profile"
echo e2e-faults-v1 > "$t/cache/ohttp.build-profile"
echo 'core=v1.0.0' > "$t/cache/set.txt"

cat > "$t/bin/git" <<'EOF'
#!/bin/sh
set -eu
if [ "$1" = init ]; then mkdir -p "$3"; exit 0; fi
[ "$1" = -C ] || exit 2
dest=$2
case "$3" in
    fetch)
        printf '%s\n' 'vauchi-protocol = { git = "https://gitlab.com/vauchi/core.git", tag = "v1.0.0", version = "=1.0.0" }' > "$dest/Cargo.toml"
        ;;
    checkout) ;;
    *) exit 2 ;;
esac
EOF
chmod +x "$t/bin/git"

for marker in missing old heads; do
    if [ "$marker" = old ]; then
        echo v1.0.0 > "$t/cache/relay.core-tag"
        echo v1.0.0 > "$t/cache/cli.core-tag"
    elif [ "$marker" = heads ]; then
        echo heads > "$t/cache/relay.core-tag"
        echo heads > "$t/cache/cli.core-tag"
    fi
    status=0
    PATH="$t/bin:$PATH" sh "$t/ci/build-pinned-binaries.sh" "$t/cache" > "$t/out" 2>&1 || status=$?
    [ "$status" = 1 ] || fail "$marker Core markers must not qualify cached binaries (exit $status)"
    grep -q "pins Core crates at another tag" "$t/out" || fail "must reject the component's actual Core pin"
    grep -qx 'core=v1.0.0' "$t/cache/set.txt" || fail "must not rewrite the receipt for a rejected cohort"
    echo "PASS: $marker Core cache markers cannot qualify another Core tag"
done

echo v2.0.0 > "$t/cache/relay.core-tag"
echo v2.0.0 > "$t/cache/cli.core-tag"
PATH="$t/bin:$PATH" sh "$t/ci/build-pinned-binaries.sh" "$t/cache" > "$t/out" 2>&1 || fail "matching Core cache markers must remain usable"
grep -qx 'core=v2.0.0' "$t/cache/set.txt" || fail "matching cohort must have an accurate receipt"
echo "PASS: matching Core cache markers retain cached binaries"
