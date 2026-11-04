#!/usr/bin/env bash
# ABOUTME: Installs a pinned Trivy on a Linux x64 runner from its GitHub release, its checksum verified, and adds it to PATH.
# ABOUTME: Replaces aquasecurity/setup-trivy, whose install script downloads from get.trivy.dev, a host that stopped resolving.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -euo pipefail

version=${1:?usage: install-trivy.sh <version, e.g. 0.70.0>}
base="https://github.com/aquasecurity/trivy/releases/download/v${version}"
tarball="trivy_${version}_Linux-64bit.tar.gz"
checksums="trivy_${version}_checksums.txt"
bin_dir="${RUNNER_TEMP:?}/trivy-bin"

download_dir=$(mktemp -d)
trap 'rm -rf "$download_dir"' EXIT
mkdir -p "$bin_dir"

curl -fsSL --retry 3 -o "${download_dir}/${tarball}" "${base}/${tarball}"
curl -fsSL --retry 3 -o "${download_dir}/${checksums}" "${base}/${checksums}"

expected=$(grep -F "  ${tarball}" "${download_dir}/${checksums}") || {
    echo "::error::${checksums} does not list ${tarball}" >&2
    exit 1
}
(cd "$download_dir" && sha256sum --check --strict - <<<"$expected")

tar -xzf "${download_dir}/${tarball}" -C "$bin_dir" trivy
"${bin_dir}/trivy" --version
echo "$bin_dir" >>"${GITHUB_PATH:?}"
