#!/usr/bin/env bash
# ABOUTME: Reads satellites.toml and moves a satellite's pins across every manifest that carries them
# ABOUTME: Subcommands resolve/sites/rewrite/verify/lock-assert/gates/names — the testable half of the bump chain

# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why the version-shaped work is here and not in YAML. YAML has no unit tests, and
# the parts of a bump lane that have actually broken are exactly the parts a test
# would have caught: the sed grammar (the photograveur lane read a manifest that no
# longer held the pin and exited at step one for weeks, 89155c33e), the verification
# strength (a negative `if TAG != VERSION` check passes while leaving every pin
# untouched), and the site discovery (a hardcoded list left one crate behind, put
# two revs in Cargo.lock and hard-failed Security Audit on main). This repo's idiom
# for that is a compile-free script with a .test.sh beside it, like
# check-lockfile-duplicates, check-async-lock-guards and check-moved-symbols.
#
# Two rules this file will not bend:
#
#   1. Substitute in place; never re-emit a line. architectural-validation.sh:852
#      extracts dravr-canot's tag with a regex requiring `{ git = "...", tag = "..."`
#      in that exact order, and enforme's line puts `version` first while
#      equilibre-sync's puts `package` first. Rebuilding a line from parsed parts
#      would normalise that order and break a CI check with a green local push.
#      satellite-pin.test.sh pins this with a byte-comparison fixture.
#
#   2. Verification is positive, never negative. After a rewrite every line
#      declaring the crate must CONTAIN the target version, or be `workspace = true`
#      (inherited, and never counted as a pin that moved). Anything else — a path
#      override, a git override on a crates.io satellite, a two-component "0.7" —
#      hard-fails with "refusing to push a partial bump". The negative form cannot
#      tell "already current" from "sed matched nothing".
#
# Exit codes: 0 success, 1 a real failure, 2 the scan could not verify anything.

set -uo pipefail

REPO_ROOT="${SATELLITE_PIN_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
TOML="${SATELLITES_TOML:-${REPO_ROOT}/satellites.toml}"

die()  { echo "❌ $*" >&2; exit 1; }
warn() { echo "⚠️  $*" >&2; }

[ -f "$TOML" ] || die "no satellites.toml at ${TOML}"

# ---------------------------------------------------------------------------
# satellites.toml reader
#
# Deliberately minimal: flat stanzas, string values, no arrays or nesting. An
# unrecognised key is an error rather than a default, so a typo fails the lane
# instead of silently selecting default behaviour.
# ---------------------------------------------------------------------------

KNOWN_KEYS=" repo source crates gates release_evidence companions upstream after_rewrite lane "

toml_get() { # $1=section $2=key -> value or empty
  awk -v want="$1" -v key="$2" '
    /^[[:space:]]*#/ { next }
    /^\[/ { section = $0; gsub(/^\[|\][[:space:]]*$/, "", section); next }
    section == want {
      line = $0
      sub(/[[:space:]]*#.*$/, "", line)
      if (match(line, /^[[:space:]]*[A-Za-z_]+[[:space:]]*=/)) {
        k = substr(line, 1, RSTART + RLENGTH - 1)
        gsub(/[[:space:]]|=/, "", k)
        if (k == key) {
          v = substr(line, RSTART + RLENGTH)
          gsub(/^[[:space:]]*"|"[[:space:]]*$/, "", v)
          print v
          exit
        }
      }
    }
  ' "$TOML"
}

toml_sections() {
  awk '/^\[/ { s = $0; gsub(/^\[|\][[:space:]]*$/, "", s); if (s != "gates") print s }' "$TOML"
}

# Fail on a key nobody reads, so a typo is loud. Runs on every invocation: this file
# is small and the alternative is a stanza that silently does the default thing.
validate_stanza() { # $1=section
  local section="$1" bad
  bad=$(awk -v want="$section" '
    /^[[:space:]]*#/ { next }
    /^\[/ { s = $0; gsub(/^\[|\][[:space:]]*$/, "", s); section = s; next }
    section == want && /=/ {
      line = $0; sub(/[[:space:]]*#.*$/, "", line)
      if (match(line, /^[[:space:]]*[A-Za-z_]+[[:space:]]*=/)) {
        k = substr(line, 1, RSTART + RLENGTH - 1); gsub(/[[:space:]]|=/, "", k); print k
      }
    }
  ' "$TOML")
  local k
  for k in $bad; do
    case "$KNOWN_KEYS" in *" $k "*) ;; *) die "satellites.toml [$section]: unknown key '$k'";; esac
  done
}

stanza_repo()     { toml_get "$1" repo; }
stanza_source()   { toml_get "$1" source; }
stanza_upstream() { toml_get "$1" upstream; }

# Which workflow moves this pin.
#
# DERIVED from the callers, not read from the declaration, because a declared answer
# goes stale silently and has: check-satellite-drift.sh's old lane table said
# dravr-stripe was moved by "tronc-bump.yml (lockstep)" and suppressed its drift on
# that basis, while this repo's tronc-bump.yml passed `lockstep_crates` without any
# stripe entry. Nothing moved that pin for as long as the table said something did.
#
# A spine caller names its satellite in one place, `satellite: <name>` under `with:`,
# so grepping for it cannot disagree with what actually runs. The `lane` key survives
# only for pins moved by a chain that is NOT this spine — contremaitre's hourly rev
# sync and tronc's producer-hosted chain — where there is no caller to read.
stanza_lane() { # $1=name
  local wf
  wf=$(grep -ls "^      satellite: $1\$" "${REPO_ROOT}"/.github/workflows/bump-*.yml 2>/dev/null | head -1)
  if [ -n "$wf" ]; then basename "$wf"; return 0; fi
  toml_get "$1" lane
}
stanza_after()    { toml_get "$1" after_rewrite; }
stanza_companions() { toml_get "$1" companions; }

stanza_crates() { # defaults to the stanza name
  local c; c=$(toml_get "$1" crates); [ -n "$c" ] && printf '%s' "$c" || printf '%s' "$1"
}

stanza_evidence() { # defaults to "release"
  local e; e=$(toml_get "$1" release_evidence); [ -n "$e" ] && printf '%s' "$e" || printf 'release'
}

stanza_gates() { # resolve aliases through [gates]; defaults to "backend"
  local aliases out a resolved
  aliases=$(toml_get "$1" gates); [ -z "$aliases" ] && aliases="backend"
  out=""
  for a in $aliases; do
    resolved=$(toml_get gates "$a")
    [ -z "$resolved" ] && die "satellites.toml [$1]: gates alias '$a' is not defined under [gates]"
    out="${out} ${resolved}"
  done
  printf '%s' "$out" | tr ' ' '\n' | grep -v '^$' | sort -u | tr '\n' ' ' | sed 's/ $//'
}

require_stanza() { # $1=name
  [ -n "$(stanza_repo "$1")" ] || die "satellites.toml has no stanza for '$1'"
  validate_stanza "$1"
}

# ---------------------------------------------------------------------------
# Manifests and pin sites
# ---------------------------------------------------------------------------

manifests() {
  # Both manifest shapes: the root [workspace.dependencies] table and per-crate
  # declarations. Never a hardcoded list — that is the failure mode this whole
  # chain exists to stop.
  { [ -f "${REPO_ROOT}/Cargo.toml" ] && echo "${REPO_ROOT}/Cargo.toml"
    find "${REPO_ROOT}/crates" -maxdepth 2 -name Cargo.toml 2>/dev/null | sort
  } 2>/dev/null
}

# Every line declaring this crate, under any local alias.
#
# A renamed dependency carries the real crate name in `package = "..."` and the key
# is only a local alias — `dravr-equilibre-sync` IS `dravr-equilibre`. Missing the
# alias is carnet#323: two copies of the crate resolve and the gate dies twenty
# minutes in as E0053. So match on the key OR on the package key, never on the key
# alone.
sites() { # $1=crate -> path<TAB>lineno<TAB>line
  local crate="$1" m
  while read -r m; do
    [ -z "$m" ] && continue
    awk -v crate="$crate" -v path="$m" '
      {
        line = $0
        if (match(line, /^[A-Za-z0-9_-]+[[:space:]]*=/)) {
          key = substr(line, 1, RSTART + RLENGTH - 1); gsub(/[[:space:]]|=/, "", key)
          declared = key
          if (match(line, /package[[:space:]]*=[[:space:]]*"[^"]+"/)) {
            pkg = substr(line, RSTART, RLENGTH)
            gsub(/package[[:space:]]*=[[:space:]]*"|"/, "", pkg)
            declared = pkg
          }
          if (declared == crate) printf "%s\t%d\t%s\n", path, NR, line
        }
      }
    ' "$m"
  done < <(manifests)
}

normalize() { printf '%s' "${1#v}"; }   # tags are v-prefixed, cargo versions are not

# ---------------------------------------------------------------------------
# resolve — what is pinned today
# ---------------------------------------------------------------------------

cmd_resolve() { # $1=name
  local name="$1" src crate first line pinned
  require_stanza "$name"
  src=$(stanza_source "$name")
  crate=$(stanza_crates "$name" | awk '{print $1}')

  first=$(sites "$crate" | grep -v 'workspace[[:space:]]*=[[:space:]]*true' | head -1)
  [ -z "$first" ] && die "no manifest pins ${crate} — refusing to report a version"
  line=$(printf '%s' "$first" | cut -f3-)

  case "$src" in
    git-tag|git-tag+version)
      pinned=$(printf '%s' "$line" | grep -oE 'tag[[:space:]]*=[[:space:]]*"v?[0-9]+\.[0-9]+\.[0-9]+"' \
               | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1) ;;
    crates-io)
      pinned=$(printf '%s' "$line" | grep -oE '(^[A-Za-z0-9_-]+[[:space:]]*=[[:space:]]*"|version[[:space:]]*=[[:space:]]*")[0-9]+\.[0-9]+\.[0-9]+"' \
               | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1) ;;
    git-rev)
      pinned=$(printf '%s' "$line" | grep -oE 'rev[[:space:]]*=[[:space:]]*"[0-9a-f]{7,40}"' \
               | grep -oE '[0-9a-f]{7,40}' | head -1) ;;
    *) die "satellites.toml [$name]: unknown source '$src'" ;;
  esac

  [ -z "$pinned" ] && die "could not read the pinned ${crate} version from: ${line}"
  printf '%s\n' "$pinned"
}

# ---------------------------------------------------------------------------
# rewrite — move every site, in place
# ---------------------------------------------------------------------------

cmd_rewrite() { # $1=name $2=target
  local name="$1" target="$2" src bare vtag crate moved=0 path
  require_stanza "$name"
  src=$(stanza_source "$name")
  bare=$(normalize "$target"); vtag="v${bare}"

  if [ "$src" = "git-rev" ]; then
    bare="$target"; vtag="$target"
    printf '%s' "$target" | grep -qE '^[0-9a-f]{7,40}$' || die "git-rev target must be a sha, got '${target}'"
  else
    printf '%s' "$bare" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' \
      || die "target must be a three-component version, got '${target}'"
  fi

  for crate in $(stanza_crates "$name"); do
    while IFS=$'\t' read -r path _ line; do
      [ -z "$path" ] && continue
      case "$line" in *"workspace = true"*|*"workspace= true"*) continue;; esac
      # Substitute in place. PIN_CRATE/PIN_NEW cross into perl through the
      # environment rather than through interpolation, so a crate name with a
      # regex metacharacter cannot alter the pattern.
      PIN_CRATE="$crate" PIN_NEW="$bare" PIN_TAG="$vtag" PIN_SRC="$src" \
      perl -i -pe '
        BEGIN { $c = quotemeta($ENV{PIN_CRATE}); $new = $ENV{PIN_NEW};
                $tag = $ENV{PIN_TAG}; $src = $ENV{PIN_SRC}; }
        # Only lines that declare this crate, under its own key or a package alias.
        next unless /^\s*[A-Za-z0-9_-]+\s*=/;
        my $decl = /package\s*=\s*"([^"]+)"/ ? $1 : (/^\s*([A-Za-z0-9_-]+)\s*=/ ? $1 : "");
        next unless $decl eq $ENV{PIN_CRATE};
        next if /workspace\s*=\s*true/;
        if ($src eq "git-rev") {
          s/(rev\s*=\s*")[0-9a-f]{7,40}(")/$1$tag$2/;
        } else {
          s/(tag\s*=\s*"v?)[0-9]+\.[0-9]+\.[0-9]+(")/${1}${new}${2}/;
          s/(version\s*=\s*")[0-9]+\.[0-9]+\.[0-9]+(")/${1}${new}${2}/;
          # Bare-string form: NAME = "0.26.0". Anchored so it cannot touch a table.
          s/^(\s*[A-Za-z0-9_-]+\s*=\s*")[0-9]+\.[0-9]+\.[0-9]+("\s*)$/${1}${new}${2}/;
        }
      ' "$path"
      moved=$(( moved + 1 ))
    done < <(sites "$crate")
  done

  [ "$moved" -eq 0 ] && die "no manifest declares any of: $(stanza_crates "$name") — nothing to rewrite"
  echo "rewrote ${moved} pin site(s) for ${name} → ${target}"
}

# ---------------------------------------------------------------------------
# verify — positive, per the rule in the header
# ---------------------------------------------------------------------------

cmd_verify() { # $1=name $2=target
  local name="$1" target="$2" src bare crate pinned inherited=0 bad=0 path lineno line
  require_stanza "$name"
  src=$(stanza_source "$name")
  bare=$(normalize "$target")
  [ "$src" = "git-rev" ] && bare="$target"
  pinned=0

  for crate in $(stanza_crates "$name"); do
    while IFS=$'\t' read -r path lineno line; do
      [ -z "$path" ] && continue
      case "$line" in
        *"workspace = true"*|*"workspace= true"*) inherited=$(( inherited + 1 )); continue;;
      esac
      if printf '%s' "$line" | grep -qF "$bare"; then
        pinned=$(( pinned + 1 ))
        # git-tag+version carries the version twice on one line; they must agree.
        if [ "$src" = "git-tag+version" ]; then
          local vcount
          vcount=$(printf '%s' "$line" | grep -oE '"v?[0-9]+\.[0-9]+\.[0-9]+"' | grep -c "${bare}\"")
          if [ "$vcount" -lt 2 ]; then
            echo "❌ ${path}:${lineno} — version and tag disagree after rewrite:"
            echo "     ${line}"
            bad=$(( bad + 1 ))
          fi
        fi
      else
        echo "❌ ${path}:${lineno} — declares ${crate} but does not carry ${bare}:"
        echo "     ${line}"
        bad=$(( bad + 1 ))
      fi
    done < <(sites "$crate")
  done

  if [ "$bad" -gt 0 ]; then
    echo "refusing to push a partial bump: ${bad} site(s) did not move to ${target}" >&2
    exit 1
  fi
  if [ "$pinned" -eq 0 ]; then
    # `workspace = true` inherits; it is never itself a pin that moved. A tree with
    # only inherited sites means the real pin was never found — the exact shape that
    # killed the photograveur lane.
    echo "❌ ${name}: ${inherited} inherited site(s) and zero real pins — the pin was never found" >&2
    exit 2
  fi
  echo "✅ ${name}: ${pinned} pin(s) at ${target}, ${inherited} inherited"
}

# ---------------------------------------------------------------------------
# lock-assert — the lockfile actually moved
# ---------------------------------------------------------------------------

cmd_lock_assert() { # $1=name $2=target
  local name="$1" target="$2" src bare crate lock found=0
  require_stanza "$name"
  src=$(stanza_source "$name")
  bare=$(normalize "$target")
  lock="${REPO_ROOT}/Cargo.lock"
  [ -f "$lock" ] || die "no Cargo.lock at ${lock}"

  for crate in $(stanza_crates "$name"); do
    case "$src" in
      crates-io)
        # A package block whose name is this crate and whose version is the target.
        if awk -v c="$crate" -v v="$bare" '
             /^name = /    { n = $0; gsub(/name = "|"/, "", n) }
             /^version = / { ver = $0; gsub(/version = "|"/, "", ver);
                             if (n == c && ver == v) { found = 1; exit } }
             END { exit !found }' "$lock"; then
          found=$(( found + 1 ))
        else
          die "Cargo.lock has no ${crate} ${bare} — the lockfile did not move"
        fi ;;
      git-tag|git-tag+version)
        if grep -qE "source = \"git\+[^\"]*${crate}\.git\?tag=v?${bare}#" "$lock"; then
          found=$(( found + 1 ))
        else
          die "Cargo.lock has no ${crate} at tag ${bare} — the lockfile did not move"
        fi ;;
      git-rev)
        if grep -qE "source = \"git\+[^\"]*${crate}\.git\?rev=${target}#" "$lock"; then
          found=$(( found + 1 ))
        else
          die "Cargo.lock has no ${crate} at rev ${target} — the lockfile did not move"
        fi ;;
    esac
  done
  echo "✅ Cargo.lock carries ${found} ${name} entry/entries at ${target}"
}

# ---------------------------------------------------------------------------

usage() {
  cat <<USAGE
usage: satellite-pin.sh <command> [args]

  names                        every satellite stanza
  sites <crate>                manifests declaring the crate (path/line/text)
  resolve <name>               the version pinned today
  rewrite <name> <target>      move every pin site, in place
  verify  <name> <target>      positive verification of every site
  lock-assert <name> <target>  Cargo.lock actually moved
  gates <name>                 workflow files this satellite must gate on
  crates <name>                package names moving on one version
  repo <name> | source <name> | upstream <name> | lane <name> |
  evidence <name> | companions <name> | after-rewrite <name>
USAGE
}

case "${1:-}" in
  names)         toml_sections ;;
  sites)         sites "${2:?crate}" ;;
  resolve)       cmd_resolve "${2:?name}" ;;
  rewrite)       cmd_rewrite "${2:?name}" "${3:?target}" ;;
  verify)        cmd_verify  "${2:?name}" "${3:?target}" ;;
  lock-assert)   cmd_lock_assert "${2:?name}" "${3:?target}" ;;
  gates)         require_stanza "${2:?name}"; stanza_gates "$2"; echo ;;
  crates)        require_stanza "${2:?name}"; stanza_crates "$2"; echo ;;
  repo)          require_stanza "${2:?name}"; stanza_repo "$2" ;;
  source)        require_stanza "${2:?name}"; stanza_source "$2" ;;
  upstream)      require_stanza "${2:?name}"; stanza_upstream "$2" ;;
  lane)          require_stanza "${2:?name}"; stanza_lane "$2" ;;
  evidence)      require_stanza "${2:?name}"; stanza_evidence "$2" ;;
  companions)    require_stanza "${2:?name}"; stanza_companions "$2" ;;
  after-rewrite) require_stanza "${2:?name}"; stanza_after "$2" ;;
  ""|-h|--help)  usage ;;
  *)             usage; exit 1 ;;
esac
