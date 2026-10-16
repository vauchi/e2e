#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail
ref=${1:?Usage: fetch-scripts.sh REF DESTINATION}
destination=${2:?Usage: fetch-scripts.sh REF DESTINATION}

if [ ! -d "$destination/.git" ]; then
    git init "$destination"
    git -C "$destination" remote add origin https://gitlab.com/vauchi/scripts.git
fi
git -C "$destination" fetch --depth=1 origin "$ref"
git -C "$destination" checkout --detach FETCH_HEAD
