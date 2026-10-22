#!/usr/bin/env bash
# ABOUTME: Maps pierre-server test suites to the binary that runs them: `tests/it` modules or a top-level file
# ABOUTME: One answer for CI shards, the changed-files lane, pre-push and coverage instead of one per script
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# Most server suites are modules of one test binary, `tests/it/main.rs`; a suite
# that writes the process environment stays a top-level file and its own binary
# (`it/test_layout_test.rs` holds both sides of that rule). A suite is named by
# its file stem either way, so a list of suites, a changed path or a shard
# assignment reads the same; this script says how each one is built and run.
#
# Usage:
#   server-suites.sh list                 every suite: "<name>\t<it|bin>", sorted by name
#   server-suites.sh targets <suite>...   the cargo --test targets they need, deduplicated
#   server-suites.sh changed-targets      the --test targets a list of changed paths (stdin,
#                                         one per line) can change the outcome of: EVERY target
#                                         when a path can reach the server's build or what its
#                                         tests read, else the edited test files' own targets
#                                         (see "Impact" below); docs and client-only paths name
#                                         none
#   server-suites.sh changed-targets --edited-only
#                                         only the targets whose test SOURCE the paths edit:
#                                         `it` for tests/it/, tests/helpers/ or common.rs, a
#                                         top-level file's own name; every other path names
#                                         none. The lint scope, not the run scope (below)
#   server-suites.sh it-args <module>...  the libtest arguments that run exactly those `it`
#                                         modules: a `<module>::` filter each, plus a
#                                         `--skip <other>::` for every other module whose
#                                         name ends in one of them
#   server-suites.sh run [--threads N] <suite>... [-- <cargo args>]
#                                         run them: each top-level suite as its own
#                                         binary, every `it` module in one `--test it`
#                                         run filtered by `<module>::`
#   server-suites.sh run --binaries <map> [--threads N] <suite>...
#                                         the same runs, from binaries a build job
#                                         already made: <map> is test-binaries.sh's
#                                         "<target>\t<package dir>\t<executable>",
#                                         and each suite execs pierre-server's binary
#                                         of that target from pierre-server's
#                                         directory, as cargo would. A suite with no
#                                         such binary fails rather than passing unrun
#
# A suite may be given as a name or as a path to its file. An unknown suite is
# an error: a caller that names a suite expects it to run.
#
# libtest filters by substring, so `goal_test::` alone also selects
# `season_goal_test::…`, which then runs on two shards. it-args adds a
# `--skip season_goal_test::` for such a module unless it is wanted too; when a
# skip would also hit a wanted module, it is left out and the extra run stays,
# so a skip never costs a test.
#
# Impact. A test's outcome changes when the code it links changes, not only
# when its own file does: a change under any crate's src/ in the server's
# dependency closure reaches every server test binary, and the lanes that run
# only "this push's tests" ran none of them for it. So changed-targets answers
# with the full target set when a path is
#   - inside a workspace crate that pierre_mcp_server reaches through path
#     dependencies of any kind (computed from `cargo metadata --no-deps`, so a
#     crate added or extracted tomorrow is classified without editing this
#     file) — except that crate's top-level *.md and the .rs sources of its own
#     tests/, benches/ and examples/ targets, which the leaf-crate lane runs;
#   - a non-.rs file under pierre-server's tests/ (a fixture some test reads);
#   - a workspace manifest, the lockfile, the toolchain pin or cargo config;
#   - a tree the server or its tests read from outside crates/: the migrations
#     (both crates/pierre-database/migrations* are symlinks to them and the
#     server embeds them), templates/, commands/, sdk/, test_data/,
#     infra/environments/, packages/{i18n,shared-constants,mcp-types};
#   - anything not classified at all. Unknown fails toward running more.
# Docs, the clients, CI configuration and scripts name none. When cargo
# metadata cannot answer, the closure is unknown and the answer is the full
# set, with a warning on stderr: planning is an optimisation, so a planner
# failure must restore the whole run, never shrink it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${SERVER_SUITES_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
TESTS="$ROOT/crates/pierre-server/tests"

die() {
    echo "server-suites.sh: $*" >&2
    exit 2
}

# The `it` modules are what it/main.rs declares, not the files beside it:
# shared support modules (helpers, common, fixtures) are compiled in from
# elsewhere and carry tests of their own, and a module that is declared runs.
it_modules() {
    sed -n 's/^ *\(pub \)\{0,1\}mod \([a-z0-9_]*\);.*$/\2/p' "$TESTS/it/main.rs"
}

list_suites() {
    {
        find "$TESTS" -maxdepth 1 -name '*.rs' ! -name common.rs -exec basename {} .rs \; \
            | sed 's/$/\tbin/'
        it_modules | sed 's/$/\tit/'
    } | LC_ALL=C sort -u
}

# Prints "it" or "bin" for one suite name or path; fails on an unknown one.
kind_of() {
    local name
    name="$(basename "$1" .rs)"
    # Not a pipe: grep -q stops reading at its match, and pipefail would
    # count the writer's SIGPIPE as the test failing.
    if grep -qxF "$name" < <(it_modules); then
        echo it
    elif [ -f "$TESTS/$name.rs" ] && [ "$name" != common ]; then
        echo bin
    else
        die "no pierre-server suite named '$name'"
    fi
}

targets() {
    local suite kind name
    for suite in "$@"; do
        kind="$(kind_of "$suite")"
        name="$(basename "$suite" .rs)"
        if [ "$kind" = it ]; then echo it; else echo "$name"; fi
    done | LC_ALL=C sort -u
}

# Name-based on purpose: a path the diff deleted still names the target it
# was part of, and the caller decides whether that target still exists.
edited_targets() {
    grep -E '^crates/pierre-server/tests/([^/]+|(it|helpers)/.+)\.rs$' \
        | sed -E 's#^crates/pierre-server/tests/((it|helpers)/.*|common\.rs)$#it#; s#.*/##; s#\.rs$##' \
        | LC_ALL=C sort -u || true
}

# Every target the suite list needs: `it` once, plus each top-level binary.
all_targets() {
    local listing
    listing="$(list_suites)"
    [ -n "$listing" ] || die "no server suites listed — refusing to answer an empty full set"
    awk -F'\t' '{ if ($2 == "it") print "it"; else print $1 }' <<< "$listing" | LC_ALL=C sort -u
}

# Prints one line per path, "<full|tests|none>\t<path>\t<why>". $1 is a file
# holding `cargo metadata --no-deps` JSON, or empty when metadata failed.
classify_paths() {
    python3 -c '
import json, os, re, sys

meta_file = sys.argv[1]
paths = [line.rstrip("\n") for line in sys.stdin if line.strip()]

SERVER = "pierre_mcp_server"
FULL_PREFIXES = (
    "Cargo.toml", "Cargo.lock", "rust-toolchain", ".cargo/", "build.rs", "clippy.toml",
    "migrations/", "migrations_pg/", "templates/", "commands/", "sdk/", "test_data/",
    "infra/environments/", "packages/i18n/", "packages/shared-constants/",
    "packages/mcp-types/",
    # pierre-contremaitre reads it through CARGO_MANIFEST_DIR.
    "scripts/ci/use-case-predicates.txt",
)
NONE_PREFIXES = (
    "docs/", "frontend/", "frontend-mobile/", "packages/", ".github/", "scripts/",
    "bin/", "load-tests/", "docker/", "infra/", "examples/", "benches/",
    ".claude/", ".agents/", ".build", ".savoir/", ".pi/", ".vscode/",
)
NONE_FILES = {
    "bun.lock", "package.json", ".gitignore", ".gitmodules", ".dockerignore",
    ".trivyignore", ".env.postgres.example", ".envrc.example", ".mcp.json",
    ".nvmrc", "deny.toml", "docker-compose.yml", "feature-phases.yaml",
    "registre.toml", "satellites.toml", "validation-patterns.local.toml",
}

def emit(kind, path, why):
    print(f"{kind}\t{path}\t{why}")

crates = {}      # member dir relative to the workspace root -> package name
closure = None   # the names pierre_mcp_server reaches, None when unknown
if meta_file:
    meta = json.load(open(meta_file))
    ws_root = meta["workspace_root"]
    by_name = {}
    for pkg in meta["packages"]:
        rel = os.path.relpath(os.path.dirname(pkg["manifest_path"]), ws_root)
        crates[rel] = pkg["name"]
        by_name[pkg["name"]] = pkg
    if SERVER in by_name:
        closure, todo = set(), [SERVER]
        while todo:
            name = todo.pop()
            if name in closure:
                continue
            closure.add(name)
            # Every kind: dev-dependencies build into the test binaries too.
            todo += [d["name"] for d in by_name[name]["dependencies"]
                     if d.get("path") and d["name"] in by_name]
server_dir = next((d for d, n in crates.items() if n == SERVER), None)

for path in paths:
    if closure is None:
        emit("full", path, "the dependency closure is unknown")
        continue
    owner = max((d for d in crates if path.startswith(d + "/")), key=len, default=None)
    if owner is not None:
        rest = path[len(owner) + 1:]
        name = crates[owner]
        if owner == server_dir and rest.startswith("tests/"):
            if re.fullmatch(r"tests/([^/]+|(it|helpers)/.+)\.rs", rest):
                emit("tests", path, "a server test source")
            else:
                emit("full", path, "a server test-tree file no target is named for")
        elif name not in closure:
            emit("none", path, f"{name} is outside the {SERVER} dependency closure")
        elif "/" not in rest and rest.endswith(".md"):
            emit("none", path, "crate documentation")
        elif re.fullmatch(r"(tests|benches|examples)/.+\.rs", rest):
            emit("none", path, f"a target of {name} itself")
        else:
            emit("full", path, f"{name} is in the {SERVER} dependency closure")
    elif path.startswith("crates/"):
        emit("full", path, "under crates/ but in no workspace member")
    elif path.startswith(FULL_PREFIXES):
        emit("full", path, "the server builds from it or its tests read it")
    elif (path.startswith(NONE_PREFIXES) or path in NONE_FILES
          or path.startswith("LICENSE") or ("/" not in path and path.endswith(".md"))):
        emit("none", path, "docs, clients, scripts or CI configuration")
    else:
        emit("full", path, "unclassified")
' "$1"
}

changed_targets() {
    if [ "${1:-}" = --edited-only ]; then
        edited_targets
        return
    fi
    [ $# -eq 0 ] || die "changed-targets: unknown argument '$1'"
    local paths meta_file="" classified full meta
    paths="$(cat)"
    [ -n "$paths" ] || return 0

    # Gated on its own exit status, never piped into its reader: a failed or
    # empty metadata run must reach the warning, not parse as no members.
    if meta="$(cd "$ROOT" && cargo metadata --format-version 1 --no-deps --offline 2>/dev/null)" \
        && [ -n "$meta" ]; then
        meta_file="$(mktemp)"
        printf '%s\n' "$meta" >"$meta_file"
    else
        echo "server-suites.sh: WARNING: cargo metadata failed, so the dependency closure is unknown — answering with every target" >&2
    fi
    if ! classified="$(classify_paths "$meta_file" <<< "$paths")"; then
        [ -z "$meta_file" ] || rm -f "$meta_file"
        echo "server-suites.sh: WARNING: path classification failed — answering with every target" >&2
        all_targets
        return
    fi
    [ -z "$meta_file" ] || rm -f "$meta_file"
    full="$(awk -F'\t' '$1 == "full"' <<< "$classified")"
    if [ -n "$full" ]; then
        # The first few reasons only: a crate change lists every file it touched.
        awk -F'\t' 'NR <= 5 { printf "server-suites.sh: every target: %s (%s)\n", $2, $3 }' <<< "$full" >&2
        all_targets
    else
        awk -F'\t' '$1 == "tests" { print $2 }' <<< "$classified" | edited_targets
    fi
}

it_args() {
    [ $# -gt 0 ] || die "it-args: no modules named"
    local module modules=()
    for module in "$@"; do
        [ "$(kind_of "$module")" = it ] || die "it-args: '$module' is not a module of the it binary"
        modules+=("$(basename "$module" .rs)")
    done
    list_suites | awk -F'\t' '$2 == "it" { print $1 }' | python3 -c '
import sys
wanted = sys.argv[1:]
everyone = [line.strip() for line in sys.stdin if line.strip()]
chosen = set(wanted)
for name in wanted:
    print(name + "::")
for other in everyone:
    if other in chosen:
        continue
    if not any(other.endswith(name) for name in chosen):
        continue
    # The skip is a substring match too: it must not reach a wanted module.
    if any(name != other and name.endswith(other) for name in chosen):
        continue
    print("--skip")
    print(other + "::")
' "${modules[@]}"
}

# Execs pierre-server's binaries of <target> from <map>, with the remaining
# arguments, through the one runner test-binaries.sh keeps for every lane that
# runs prebuilt binaries. From the workspace root, so its `Running` line names
# the source as cargo does; <map> is absolute by then.
run_prebuilt() {
    local map="$1" target="$2"
    shift 2
    (cd "$ROOT" && "$SCRIPT_DIR/test-binaries.sh" run "$map" "$ROOT/crates/pierre-server" "$target" "$@")
}

run() {
    local threads=4 binaries=""
    local suites=() cargo_args=()
    while [ $# -gt 0 ]; do
        case "$1" in
            --threads) threads="$2"; shift 2 ;;
            --binaries) binaries="$2"; shift 2 ;;
            --) shift; cargo_args=("$@"); break ;;
            *) suites+=("$1"); shift ;;
        esac
    done
    [ "${#suites[@]}" -gt 0 ] || die "run: no suites named"
    if [ -n "$binaries" ]; then
        [ -s "$binaries" ] || die "run: --binaries $binaries is missing or empty"
        [ "${#cargo_args[@]}" -eq 0 ] || die "run: cargo arguments mean nothing to prebuilt binaries"
        binaries="$(cd "$(dirname "$binaries")" && pwd)/$(basename "$binaries")"
    fi

    local bins=() modules=() filters=() suite name
    for suite in "${suites[@]}"; do
        name="$(basename "$suite" .rs)"
        if [ "$(kind_of "$suite")" = it ]; then
            modules+=("$name")
        else
            bins+=("$name")
        fi
    done

    local failed=0
    for name in "${bins[@]+"${bins[@]}"}"; do
        group "$name"
        if [ -n "$binaries" ]; then
            if ! run_prebuilt "$binaries" "$name" --test-threads="$threads"; then
                report "suite $name failed"
                failed=1
            fi
        elif ! cargo test "${cargo_args[@]+"${cargo_args[@]}"}" --test "$name" -- --test-threads="$threads"; then
            report "suite $name failed"
            failed=1
        fi
        endgroup
    done
    if [ "${#modules[@]}" -gt 0 ]; then
        # A command substitution, not `< <(...)`: a failed it_args must stop the
        # run, never leave the filters empty and run the whole binary.
        local filter_lines
        filter_lines="$(it_args "${modules[@]}")" || die "it-args failed for: ${modules[*]}"
        mapfile -t filters <<< "$filter_lines"
        group "it (${#modules[@]} suites)"
        if [ -n "$binaries" ]; then
            if ! run_prebuilt "$binaries" it --test-threads="$threads" "${filters[@]}"; then
                report "a suite in the it binary failed: ${filters[*]}"
                failed=1
            fi
        elif ! cargo test "${cargo_args[@]+"${cargo_args[@]}"}" --test it -- --test-threads="$threads" "${filters[@]}"; then
            report "a suite in the it binary failed: ${filters[*]}"
            failed=1
        fi
        endgroup
    fi
    return "$failed"
}

group() { if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::group::$1"; else echo "=== $1"; fi; }
endgroup() { if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::endgroup::"; fi; }
report() { if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$1"; else echo "FAILED: $1" >&2; fi; }

command="${1:-}"
[ $# -gt 0 ] && shift
# Every command but changed-targets --edited-only reads the tree; that one maps names only.
if [ "$command" != changed-targets ] || [ "${1:-}" != --edited-only ]; then
    [ -d "$TESTS/it" ] || die "$TESTS/it is missing"
fi
case "$command" in
    list) list_suites ;;
    targets) [ $# -gt 0 ] || die "targets: no suites named"; targets "$@" ;;
    changed-targets) changed_targets "$@" ;;
    it-args) it_args "$@" ;;
    run) run "$@" ;;
    *) die "usage: server-suites.sh list | targets <suite>... | changed-targets [--edited-only] < paths | it-args <module>... | run [--binaries <map>] [--threads N] <suite>... [-- <cargo args>]" ;;
esac
