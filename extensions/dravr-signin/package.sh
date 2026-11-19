#!/usr/bin/env bash
# ABOUTME: Packs the extension into the zip the Chrome Web Store, Edge Add-ons and addons.mozilla.org take
# ABOUTME: Ships only what the browser runs: no tests, no README, no store material (carnet#1617)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage: extensions/dravr-signin/package.sh [out-dir]   (default: extensions/dravr-signin/dist)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/dist}"
version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$here/manifest.json")"
[ -n "$version" ] || { echo "no version in manifest.json" >&2; exit 1; }

mkdir -p "$out"
zip_path="$(cd "$out" && pwd)/dravr-signin-$version.zip"
rm -f "$zip_path"

cd "$here"
files=(manifest.json background.js bridge.js signin.js notice.html notice.js)
for file in "${files[@]}"; do [ -f "$file" ] || { echo "missing $file" >&2; exit 1; }; done
# A hidden file (a Finder .DS_Store) never ships: the stores' linters flag it.
zip -qrX "$zip_path" "${files[@]}" icons _locales -x '.*' '*/.*'
echo "$zip_path"
