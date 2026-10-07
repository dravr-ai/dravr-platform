#!/usr/bin/env bash
# ABOUTME: Fixture test for check-contremaitre-sync.sh — Check 3's tool list from the one registry scan,
# ABOUTME: Check 1's server-rendered keys read from contremaitre, and Check 8's bot-only pin
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Check 3 used to grep `tool_definition(` itself, beside the scan Check 7 and
# check-declared-parameters.sh attribute properties with: two enumerations of
# one registry, each with its own "scan incomplete" guard (carnet#583). A tree
# that broke the shared scan's premise then read as "all tools in sync" to
# Check 3, and the only gate that noticed was Check 7 — which skips whenever the
# contremaitre corpus cannot be resolved. Case 2 is that shape, and it passes
# against the grep.
#
# The script anchors on its own location, so each fixture hosts copies of it
# and of tool_schema_properties.py. The fixture is only what Check 3 reads:
# Check 1 and the corpus checks fail or skip on it, so this asserts on Check 3's
# lines rather than on the exit code. `cargo` is stubbed off PATH — the script
# asks `cargo metadata` for the pinned corpus, and nothing here needs it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-contremaitre-sync.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
OUT="$TMP/sync.out"

mkdir -p "$TMP/bin"
printf '#!/bin/sh\nexit 1\n' > "$TMP/bin/cargo"
chmod +x "$TMP/bin/cargo"

# tree <name> — two tools, and the three mirrors Check 3 compares them with.
tree() {
    local root="$TMP/$1"
    mkdir -p "$root/scripts/ci" "$root/crates/demo/src" "$root/crates/pierre-server/tests" \
             "$root/packages/mcp-types/src"
    cp "$UNDER_TEST" "$root/scripts/ci/check-contremaitre-sync.sh"
    cp "$SCRIPT_DIR/tool_schema_properties.py" "$root/scripts/ci/tool_schema_properties.py"
    cat > "$root/crates/demo/src/tools.rs" <<'RS'
impl McpTool for Alpha {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert("days", PropertySchema::default());
        tool_definition("alpha_tool", "Alpha", object_schema(properties, None))
    }
}

impl McpTool for Beta {
    fn definition(&self) -> Tool {
        tool_definition("beta_tool", "Beta", object_schema(HashMap::new(), None))
    }
}
RS
    printf 'const EXPECTED_TOOLS: &[&str] = &[\n    "alpha_tool",\n    "beta_tool",\n];\n' \
        > "$root/crates/pierre-server/tests/contremaitre_test.rs"
    printf 'fn count() {\n    assert_eq!(tools.len(), 2);\n}\n' \
        > "$root/crates/pierre-server/tests/configuration_mcp_integration_test.rs"
    printf '// Tool count: 2\nexport type ToolName = "alpha_tool" | "beta_tool";\n' \
        > "$root/packages/mcp-types/src/tools.ts"
    echo "$root"
}

run() { ( cd "$1" && PATH="$TMP/bin:$PATH" bash scripts/ci/check-contremaitre-sync.sh ) >"$OUT" 2>&1 || true; }

expect_output() { # $1 = label, $2 = literal the last run must print
    if grep -qF -- "$2" "$OUT"; then pass "$1"; else
        fail "$1 (output is missing: $2)"
        sed 's/^/      /' "$OUT"
    fi
}

expect_absent() { # $1 = label, $2 = literal the last run must NOT print
    if grep -qF -- "$2" "$OUT"; then
        fail "$1 (output carries: $2)"
        sed 's/^/      /' "$OUT"
    else pass "$1"; fi
}

echo "==== check-contremaitre-sync.sh fixture test ===="

# 1. The baseline: two tools, three mirrors in agreement.
root="$(tree baseline)"
run "$root"
expect_output "two registered tools match their mirrors" "MCP tool list: 2 tools match EXPECTED_TOOLS"

# 2. A property declared after the final tool_definition. The shared scan
#    refuses it (its attribution would read the schema as empty), and Check 3
#    reads that same scan, so it refuses too instead of reporting a clean list.
root="$(tree ordering)"
cat >> "$root/crates/demo/src/tools.rs" <<'RS'

fn late(properties: &mut HashMap<String, PropertySchema>) {
    properties.insert("late", PropertySchema::default());
}
RS
run "$root"
expect_output "a scan the shared enumeration refuses fails Check 3" "Tool scan incomplete"
expect_output "the refusal carries the shared scan's reason" "schema-before-definition ordering is violated"
expect_absent "no clean tool list is reported over a refused scan" "MCP tool list: 2 tools match"

# 3. Two call sites registering one name.
root="$(tree duplicate)"
cat >> "$root/crates/demo/src/tools.rs" <<'RS'

impl McpTool for Gamma {
    fn definition(&self) -> Tool {
        tool_definition("beta_tool", "Gamma", object_schema(HashMap::new(), None))
    }
}
RS
run "$root"
expect_output "a name registered twice fails Check 3 by that name" "so a name is registered twice"

# 4. A computed name the scan cannot read.
root="$(tree computed)"
cat >> "$root/crates/demo/src/tools.rs" <<'RS'

impl McpTool for Delta {
    fn definition(&self) -> Tool {
        tool_definition(DELTA_NAME, "Delta", object_schema(HashMap::new(), None))
    }
}
RS
run "$root"
expect_output "a computed tool name fails Check 3" "have a name this scan cannot read as a literal"

# ---------------------------------------------------------------------------
# Check 1 and Check 8 (carnet#826). The fixture grows what they read: a
# catalogue copy, the locale lists, the registry's KEY_* constants, a git
# history with an origin/main, and a contremaitre tree at ../dravr-contremaitre
# — the sibling checkout the script falls back to when cargo cannot resolve the
# pin, which is what lets these cases run without --contremaitre-root.
# ---------------------------------------------------------------------------
LOCALES=(fr en es de pt)
CM="$TMP/dravr-contremaitre"
mkdir -p "$CM/strings" "$CM/tools" "$CM/schemas" "$CM/prompts/agents/demo"
printf "# Demo\n" > "$CM/prompts/agents/demo/coach.md"
printf "description: Alpha\n" > "$CM/tools/alpha_tool.yaml"
printf 'events:\n  - name: demo_event\n' > "$CM/schemas/notify-events.yaml"
for l in "${LOCALES[@]}"; do
    printf '{"common": {"greet": "Hi {0}", "cancel": "Cancel {{name}}"}}\n' > "$CM/strings/$l.json"
done
printf '# comment\ncommon.greet\n' > "$CM/strings/server-rendered-keys.txt"

catalogue_tree() { # <name> — tree() plus everything Checks 1 and 8 read, committed as origin/main
    local root
    root="$(tree "$1")"
    mkdir -p "$root/crates/pierre-contremaitre/src" "$root/crates/pierre-core/src/models" "$root/packages/i18n/src"
    printf 'pub const SUPPORTED_LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];\n' \
        > "$root/crates/pierre-core/src/models/user.rs"
    printf "export const SUPPORTED_LANGUAGES = ['fr', 'en', 'es', 'de', 'pt'] as const;\n" \
        > "$root/packages/i18n/src/config.ts"
    printf 'pub const KEY_GREET: &str = "common.greet";\n' > "$root/crates/pierre-contremaitre/src/keys.rs"
    for l in "${LOCALES[@]}"; do
        mkdir -p "$root/packages/i18n/src/locales/$l"
        cp "$CM/strings/$l.json" "$root/packages/i18n/src/locales/$l/translation.json"
    done
    printf '[workspace.dependencies]\ndravr-contremaitre = { git = "https://github.com/dravr-ai/dravr-contremaitre.git", rev = "aaaaaaaa" }\n' \
        > "$root/Cargo.toml"
    printf '[[package]]\nname = "dravr-contremaitre"\nsource = "git+https://github.com/dravr-ai/dravr-contremaitre.git?rev=aaaaaaaa#aaaaaaaa"\n' \
        > "$root/Cargo.lock"
    git -C "$root" init -q
    git -C "$root" add -A
    git -C "$root" -c user.name=t -c user.email=t@t commit -q -m base
    git -C "$root" update-ref refs/remotes/origin/main HEAD
    echo "$root"
}
commit_all() { git -C "$1" add -A && git -C "$1" -c user.name=t -c user.email=t@t commit -q -m "$2"; }

# 5. Every KEY_* listed, one listed key also server-rendered: the baseline.
root="$(catalogue_tree catalogue)"
run "$root"
expect_output "a KEY_* listed by contremaitre passes Check 1" "Catalogue invariant: 2 keys × 5 locales, 1 server-rendered, 1 read by KEY_*"
expect_output "a branch with no pin move passes Check 8" "Pin ownership: no commit here moves"

# 6. A KEY_* contremaitre does not list: the platform got ahead of upstream.
root="$(catalogue_tree unlisted)"
printf 'pub const KEY_CANCEL: &str = "common.cancel";\n' >> "$root/crates/pierre-contremaitre/src/keys.rs"
run "$root"
expect_output "a KEY_* contremaitre does not list fails Check 1" \
    "registry declares common.cancel but contremaitre does not list it in strings/server-rendered-keys.txt"

# 7. A listed key no KEY_* reads yet: contremaitre got ahead, which is the
#    supported order and must not wedge anything.
root="$(catalogue_tree listed_early)"
printf '' > "$root/crates/pierre-contremaitre/src/keys.rs"
run "$root"
expect_output "a listed key no KEY_* reads yet passes Check 1" "1 server-rendered, 0 read by KEY_*"

# 7b. A listed key the catalogue does not carry: the list and the strings
#     beside it disagree inside contremaitre itself.
root="$(catalogue_tree listed_missing)"
printf '# comment\ncommon.greet\ncommon.gone\n' > "$CM/strings/server-rendered-keys.txt"
run "$root"
printf '# comment\ncommon.greet\n' > "$CM/strings/server-rendered-keys.txt"
expect_output "a listed key the catalogue lacks fails Check 1" \
    "server-rendered-keys.txt lists common.gone but the catalogue has no such key"

# 8. --contremaitre-root reads that tree's strings and skips the pin checks.
root="$(catalogue_tree override)"
run_override() { ( cd "$1" && PATH="$TMP/bin:$PATH" bash scripts/ci/check-contremaitre-sync.sh --contremaitre-root "$2" ) >"$OUT" 2>&1 || true; }
run_override "$root" "$CM"
expect_output "--contremaitre-root reads the named tree" "Reading the unpinned contremaitre tree at $CM/"
expect_output "--contremaitre-root skips the string copy" "String copy skipped"
expect_output "--contremaitre-root skips pin ownership" "Pin ownership skipped"
run_override "$root" "$TMP/nowhere"
expect_output "--contremaitre-root refuses a tree with no strings/" "--contremaitre-root needs a dravr-contremaitre tree"

# 9. A hand bump of the rev line fails Check 8 by naming both revs.
root="$(catalogue_tree hand_bump)"
sed -i.bak 's/rev = "aaaaaaaa"/rev = "bbbbbbbb"/' "$root/Cargo.toml" && rm "$root/Cargo.toml.bak"
commit_all "$root" "hand bump"
run "$root"
expect_output "a moved rev line fails Check 8" "Cargo.toml: dravr-contremaitre rev aaaaaaaa → bbbbbbbb"

# 9b. A hand bump of the lock entry alone fails Check 8.
root="$(catalogue_tree hand_lock)"
sed -i.bak 's/aaaaaaaa/bbbbbbbb/g' "$root/Cargo.lock" && rm "$root/Cargo.lock.bak"
commit_all "$root" "hand lock"
run "$root"
expect_output "a moved lock source fails Check 8" "Cargo.lock: the dravr-contremaitre source line"

# 9c. A rev line the pattern cannot read fails closed instead of comparing
#     two empty reads as equal.
root="$(catalogue_tree unreadable)"
sed -i.bak 's/rev = "aaaaaaaa"/tag = "v1"/' "$root/Cargo.toml" && rm "$root/Cargo.toml.bak"
commit_all "$root" "unreadable pin"
run "$root"
expect_output "an unreadable rev line fails Check 8 closed" "no dravr-contremaitre rev line at HEAD matches"
expect_absent "no clean pin ownership over an unreadable rev" "Pin ownership: no commit here moves"

# 10. A hand-edited string copy fails Check 8 by naming the file.
root="$(catalogue_tree hand_copy)"
printf '{"common": {"greet": "Hello {0}", "cancel": "Cancel {{name}}"}}\n' \
    > "$root/packages/i18n/src/locales/en/translation.json"
commit_all "$root" "hand copy"
run "$root"
expect_output "an edited string copy fails Check 8" "packages/i18n/src/locales/en/translation.json"

# 11. A commit origin/main already holds — the bump lane's own view — passes.
root="$(catalogue_tree bot_view)"
sed -i.bak 's/rev = "aaaaaaaa"/rev = "bbbbbbbb"/' "$root/Cargo.toml" && rm "$root/Cargo.toml.bak"
commit_all "$root" "chore(contremaitre): bump"
git -C "$root" update-ref refs/remotes/origin/main HEAD
run "$root"
expect_output "a bump already on origin/main passes Check 8" "Pin ownership: no commit here moves"

# 12. No origin/main to judge against fails closed rather than reading clean.
root="$(catalogue_tree no_origin)"
git -C "$root" update-ref -d refs/remotes/origin/main
run "$root"
expect_output "no origin/main fails Check 8 closed" "Pin ownership cannot be judged"

echo ""
if [[ "$failures" -gt 0 ]]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all cases passed"
