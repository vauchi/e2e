#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Sentinel tests for check-test-lane-coverage.sh. Inject a fake test
# lister (LANE_LIST_CMD) so the set-difference logic is exercised without
# compiling the e2e crate. Run: sh scripts/check-test-lane-coverage.test.sh
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
GUARD="$SCRIPT_DIR/check-test-lane-coverage.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
fails=0

# Fake lister: universe (kind(test)) = A,B,C; the integration filter is
# echoed back so a case can make it select a subset. Convention for the
# test: filter "ALL" selects A,B,C; filter "NARROW" selects A,B only.
cat >"$WORK/lister.sh" <<'LISTER'
#!/bin/sh
case "$1" in
	"kind(test)") printf 'A\nB\nC\n' ;;
	NARROW)       printf 'A\nB\n' ;;
	*)            printf 'A\nB\nC\n' ;;
esac
LISTER
chmod +x "$WORK/lister.sh"

run_guard() { # $1=filter-content $2=allowlist-content
	printf '%s\n' "$1" >"$WORK/filter"
	printf '%s\n' "$2" >"$WORK/allow"
	LANE_LIST_CMD="$WORK/lister.sh" \
	LANE_FILTER_FILE="$WORK/filter" \
	LANE_ALLOWLIST_FILE="$WORK/allow" \
		sh "$GUARD" >"$WORK/out" 2>"$WORK/err"
}

check() { # $1=label $2=expected-exit $3=actual-exit
	if [ "$2" -eq "$3" ]; then
		echo "ok   - $1"
	else
		echo "FAIL - $1 (expected exit $2, got $3)"
		sed 's/^/       /' "$WORK/err"
		fails=$((fails + 1))
	fi
}

# 1. Comprehensive filter selects everything -> pass.
set +e; run_guard 'kind(test)' ''; rc=$?; set -e
check "full filter covers all -> exit 0" 0 "$rc"

# 2. Narrowed filter drops C, no allowlist -> fail (this is the original bug).
set +e; run_guard 'NARROW' ''; rc=$?; set -e
check "narrowed filter leaves C uncovered -> exit 1" 1 "$rc"
if ! grep -q 'C' "$WORK/err"; then
	echo "FAIL - uncovered output should name C"; fails=$((fails + 1))
fi

# 3. Narrowed filter but C is allowlisted -> pass.
set +e; run_guard 'NARROW' 'C'; rc=$?; set -e
check "allowlisted C tolerates narrow filter -> exit 0" 0 "$rc"

# 4. Allowlist comments/blanks ignored; C still uncovered -> fail.
set +e; run_guard 'NARROW' '# C is fine'; rc=$?; set -e
check "commented allowlist does not cover C -> exit 1" 1 "$rc"

if [ "$fails" -eq 0 ]; then
	echo "All sentinel cases passed."
else
	echo "$fails sentinel case(s) failed."; exit 1
fi
