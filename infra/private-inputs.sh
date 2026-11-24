#!/usr/bin/env bash
# ABOUTME: Stages an environment's private terraform var-file as private.auto.tfvars in its root module
# ABOUTME: Reads it from a dravr-savoir checkout (default: the .savoir clone at the repository root)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage: infra/private-inputs.sh <dev|prod> [path to a dravr-savoir checkout]
#
# Run it before `terraform plan` or `terraform apply` in infra/environments/<env>.
# Terraform loads *.auto.tfvars by itself, after terraform.tfvars, and
# infra/.gitignore keeps the staged copy out of the tree. The script copies
# rather than links so a plan never reads a file that changes under it.
set -euo pipefail

env_name="${1:-}"
case "${env_name}" in
  dev | prod) ;;
  *)
    echo "usage: $0 <dev|prod> [dravr-savoir checkout]" >&2
    exit 2
    ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
savoir="${2:-${root}/.savoir}"
source_file="${savoir}/platform/infra/${env_name}.private.tfvars"
target_dir="${root}/infra/environments/${env_name}"

if [ ! -f "${source_file}" ]; then
  echo "error: ${source_file} not found. Refresh the private rules (bash .build/ci/savoir.sh sync) or pass a dravr-savoir checkout." >&2
  exit 1
fi
if [ ! -d "${target_dir}" ]; then
  echo "error: ${target_dir} is not a terraform environment" >&2
  exit 1
fi

install -m 600 "${source_file}" "${target_dir}/private.auto.tfvars"
echo "staged private inputs for ${env_name}: $(grep -cE '^[[:space:]]*"?[A-Za-z_][^=]*=' "${target_dir}/private.auto.tfvars") assignments"
