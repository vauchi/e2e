#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

set -eu
ref=${1:?Usage: fetch-features.sh REF DESTINATION}
destination=${2:?Usage: fetch-features.sh REF DESTINATION}
if [ ! -d "$destination/.git" ]; then
    git init "$destination"
    git -C "$destination" remote add origin https://gitlab.com/vauchi/features.git
fi
git -C "$destination" fetch --depth=1 origin "$ref"
git -C "$destination" checkout --detach FETCH_HEAD
git -C "$destination" rev-parse HEAD
