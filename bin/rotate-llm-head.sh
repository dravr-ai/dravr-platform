#!/usr/bin/env bash
# ABOUTME: Moves an Anthropic organization to the head of the LLM chain's key pool, in terraform and optionally live
# ABOUTME: One command instead of gcloud archaeology; terraform stays the source of truth so an apply keeps the order
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The chain's `anthropic` tier reads ANTHROPIC_API_KEY, `anthropic#2` reads
# ANTHROPIC_API_KEY_2, `anthropic#3` ANTHROPIC_API_KEY_3 (carnet#875). Which
# organization serves first is therefore which secret each variable is bound
# to. On 2026-11-21 that was swapped by hand on the live service; the next
# terraform apply put it back (carnet#1709, post-mortem change 7). This script
# edits the three bindings in infra/environments/dev/main.tf and, with --live,
# updates the same three variables on the running API service, so the file and
# the service agree.
#
# A spent head does not need this: the chain passes over it (quota cooldown, or
# the tier bench for a dead key). The spent organization stays the head on
# purpose (JF, 2026-11-23). Use this to choose a different head deliberately.
#
# Usage:
#   bin/rotate-llm-head.sh                    show the order in terraform and live
#   bin/rotate-llm-head.sh --head <org>       put <org> first in main.tf; the other
#                                             two keep their relative order
#   bin/rotate-llm-head.sh --head <org> --live
#                                             also update the running service now
#   --dry-run                                 print what would change, change nothing
#
# <org> is a terraform secret key (anthropic_api_key, anthropic_api_key_2,
# anthropic_api_key_3) or its Secret Manager id (dravr-mcp-server-anthropic-api-key-phil).
#
# --live touches only the three ANTHROPIC_API_KEY* variables
# (gcloud --update-secrets, never --set-secrets, which replaces every secret
# binding) and starts a new revision. Commit and push the main.tf change after,
# so the next apply keeps the order.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN_TF="$REPO_ROOT/infra/environments/dev/main.tf"
SECRETS_TF="$REPO_ROOT/infra/modules/secrets/main.tf"
PROJECT="${GCP_PROJECT:-dravr-dev}"
REGION="${GCP_REGION:-northamerica-northeast1}"
SERVICE="${API_SERVICE:-dravr-mcp-server-api}"
SERVICE_PREFIX="dravr-mcp-server"
VARS=(ANTHROPIC_API_KEY ANTHROPIC_API_KEY_2 ANTHROPIC_API_KEY_3)

die() { echo "rotate-llm-head: $*" >&2; exit 1; }

head_org=""
live=false
dry_run=false
while [ $# -gt 0 ]; do
  case "$1" in
    --head) [ $# -ge 2 ] || die "--head needs an organization"; head_org="$2"; shift 2 ;;
    --live) live=true; shift ;;
    --dry-run) dry_run=true; shift ;;
    -h|--help) sed -n '/^# Usage:/,/^[^#]/{/^#/s/^# \{0,1\}//p;}' "$0"; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

[ -f "$MAIN_TF" ] || die "no $MAIN_TF"
[ -f "$SECRETS_TF" ] || die "no $SECRETS_TF"

# The terraform key bound to a variable in a copy of main.tf (default: main.tf
# itself), e.g. anthropic_api_key_2.
bound_key() {
  local var="$1" file="${2:-$MAIN_TF}" line
  line="$(grep -E "^[[:space:]]*${var}[[:space:]]*=[[:space:]]*module\.secrets\.secret_ids\[\"[a-z0-9_]+\"\]" "$file" || true)"
  [ "$(printf '%s\n' "$line" | grep -c .)" -eq 1 ] || die "expected exactly one binding of $var in main.tf"
  printf '%s\n' "$line" | sed -E 's/.*secret_ids\["([a-z0-9_]+)"\].*/\1/'
}

# The Secret Manager id of a terraform key, read from the secrets module.
secret_id_of() {
  local key="$1" suffix
  suffix="$(awk -v key="$key" '
    $0 ~ "^resource \"google_secret_manager_secret\" \"" key "\"" { inside = 1; next }
    inside && /^}/ { inside = 0 }
    inside && /secret_id/ { sub(/.*\$\{var\.service_name\}-/, ""); sub(/".*/, ""); print; exit }
  ' "$SECRETS_TF")"
  [ -n "$suffix" ] || die "no secret_id for $key in the secrets module"
  printf '%s-%s\n' "$SERVICE_PREFIX" "$suffix"
}

# Resolved up front by array appends, which `set -e` stops on; a lookup inside
# printf or `[ ]` would print its error and carry on with an empty id.
current=()
for var in "${VARS[@]}"; do current+=("$(bound_key "$var")"); done
current_ids=()
for key in "${current[@]}"; do current_ids+=("$(secret_id_of "$key")"); done

echo "Terraform (main.tf):"
for i in 0 1 2; do
  printf '  %-20s -> %-22s (%s)\n' "${VARS[$i]}" "${current[$i]}" "${current_ids[$i]}"
done

# One secret id per variable, in VARS order; empty when the service cannot be read.
live_bindings=""
if command -v gcloud >/dev/null 2>&1; then
  live_bindings="$(gcloud run services describe "$SERVICE" --project "$PROJECT" --region "$REGION" \
    --format=json 2>/dev/null | python3 -c '
import json, sys
env = json.load(sys.stdin)["spec"]["template"]["spec"]["containers"][0].get("env", [])
for name in sys.argv[1:]:
    ref = next((e.get("valueFrom", {}).get("secretKeyRef", {}) for e in env if e["name"] == name), {})
    print(ref.get("name", "(not bound)"))
' "${VARS[@]}" 2>/dev/null || true)"
  if [ -n "$live_bindings" ]; then
    echo "Live ($SERVICE):"
    i=0
    while IFS= read -r secret; do
      printf '  %-20s -> %s\n' "${VARS[$i]}" "$secret"
      i=$((i + 1))
    done <<<"$live_bindings"
  else
    echo "Live: not read (gcloud not authenticated, or no access to $PROJECT)"
  fi
fi

[ -n "$head_org" ] || exit 0

# Resolve <org> to a terraform key.
target=""
target_id=""
for i in 0 1 2; do
  if [ "$head_org" = "${current[$i]}" ] || [ "$head_org" = "${current_ids[$i]}" ]; then
    target="${current[$i]}"
    target_id="${current_ids[$i]}"
  fi
done
[ -n "$target" ] || die "unknown organization '$head_org': give one of ${current[*]} or its Secret Manager id"

new=("$target")
new_ids=("$target_id")
for i in 0 1 2; do
  [ "${current[$i]}" = "$target" ] && continue
  new+=("${current[$i]}")
  new_ids+=("${current_ids[$i]}")
done

edit_tf=true
if [ "${new[*]}" = "${current[*]}" ]; then
  edit_tf=false
  # A service changed by hand drifts from main.tf; --live still brings it back.
  live_matches=false
  [ "$live_bindings" = "$(printf '%s\n' "${new_ids[@]}")" ] && live_matches=true
  if ! $live || $live_matches; then
    echo "$target is already the head; nothing to do."
    exit 0
  fi
  echo "$target is already the head in main.tf; the live service differs or could not be read."
fi

echo "New order:"
for i in 0 1 2; do
  printf '  %-20s -> %-22s (%s)\n' "${VARS[$i]}" "${new[$i]}" "${new_ids[$i]}"
done

if $dry_run; then
  echo "--dry-run: nothing changed."
  exit 0
fi

if $edit_tf; then
  # Edit and check a copy, so a failed rewrite leaves main.tf untouched.
  tmp="$(mktemp)"
  trap 'rm -f "$tmp" "$tmp.bak"' EXIT
  cp "$MAIN_TF" "$tmp"
  for i in 0 1 2; do
    sed -E -i.bak "s/^([[:space:]]*${VARS[$i]}[[:space:]]*=[[:space:]]*module\.secrets\.secret_ids\[\")[a-z0-9_]+(\"\])/\1${new[$i]}\2/" "$tmp"
  done
  for i in 0 1 2; do
    [ "$(bound_key "${VARS[$i]}" "$tmp")" = "${new[$i]}" ] || die "the rewrite did not take the new binding of ${VARS[$i]}; main.tf is unchanged"
  done
  cat "$tmp" > "$MAIN_TF"
  echo "Updated $MAIN_TF."
fi

if $live; then
  command -v gcloud >/dev/null 2>&1 || die "--live needs gcloud"
  updates=""
  for i in 0 1 2; do
    updates="${updates:+$updates,}${VARS[$i]}=${new_ids[$i]}:latest"
  done
  echo "Updating $SERVICE live (new revision): $updates"
  gcloud run services update "$SERVICE" --project "$PROJECT" --region "$REGION" --update-secrets "$updates" \
    || die "the live update failed (is gcloud authenticated for $PROJECT?); main.tf already has the new order"
fi

cat <<EOF

Next:
  1. Commit infra/environments/dev/main.tf and push it to main, so the next
     terraform apply keeps this order.
EOF
if ! $live; then
  cat <<EOF
  2. Apply it: gh workflow run terraform.yml -f environment=dev -f action=apply
     (or rerun this script with --live to change the running service now).
EOF
fi
