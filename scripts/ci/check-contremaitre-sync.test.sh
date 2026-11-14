#!/usr/bin/env bash
# ABOUTME: Fixture test for check-contremaitre-sync.sh — Check 3's tool list from the one registry scan,
# ABOUTME: Check 1's server-rendered keys read from contremaitre, Check 8's bot-only pin, and Check 4's framing scan of every catalogue string in all five locales
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

# served <root> <json> — the admin tools/list fixture: what /mcp serves.
served() {
    printf '%s\n' "$2" > "$1/crates/pierre-server/tests/fixtures/mcp_surface/tools_list_admin.json"
}

# public <root> <json> — the public profile's tool names: what the SDK types describe.
public() {
    printf '%s\n' "$2" > "$1/crates/pierre-server/tests/fixtures/mcp_surface/tools_list_public_names.json"
}

# tree <name> — two tools, and the three mirrors Check 3 compares them with.
tree() {
    local root="$TMP/$1"
    mkdir -p "$root/scripts/ci" "$root/crates/demo/src" "$root/crates/pierre-server/tests/it" \
             "$root/crates/pierre-server/tests/fixtures/mcp_surface" "$root/packages/mcp-types/src"
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
        > "$root/crates/pierre-server/tests/it/contremaitre_test.rs"
    printf 'fn count() {\n    assert_eq!(tools.len(), 2);\n}\n' \
        > "$root/crates/pierre-server/tests/it/configuration_mcp_integration_test.rs"
    printf '// Tool count: 2\nexport type ToolName = "alpha_tool" | "beta_tool";\n' \
        > "$root/packages/mcp-types/src/tools.ts"
    served "$root" '{"tools":[{"name":"alpha_tool"},{"name":"beta_tool"}]}'
    public "$root" '["alpha_tool","beta_tool"]'
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

# 0. The script parses under /bin/bash, the shell its shebang names. Every case
# below runs it through `bash`, which on a Mac is Homebrew's 5.x when that is
# first on PATH, while pre-push runs the shebang's 3.2: an unpaired quote
# inside a heredoc passes one and fails the other. On Linux /bin/bash is 5.x
# and this proves the same as the cases below.
if /bin/bash -n "$UNDER_TEST" 2>"$OUT"; then
    pass "the script parses under /bin/bash"
else
    fail "the script parses under /bin/bash"
    sed 's/^/      /' "$OUT"
fi

# 1. The baseline: two tools, three mirrors in agreement.
root="$(tree baseline)"
run "$root"
expect_output "two registered tools match their mirrors" "MCP tool list: 2 tools match EXPECTED_TOOLS"

# 1b. The SDK types mirror what /mcp serves, not every registered tool: a tool
#     kept to Dravr's own surfaces is registered, unserved and absent from them.
root="$(tree first_party)"
served "$root" '{"tools":[{"name":"alpha_tool"}]}'
public "$root" '["alpha_tool"]'
printf '// Tool count: 1\nexport type ToolName = "alpha_tool";\n' > "$root/packages/mcp-types/src/tools.ts"
run "$root"
expect_output "SDK types may leave out a registered tool /mcp does not serve" "the SDK types match the 1 tools of the public profile"

# 1c. A coach-only tool is served to a coach but not to the public profile the
#     SDK types describe (carnet#1426): types without it pass, types with it fail.
root="$(tree coach_only)"
public "$root" '["alpha_tool"]'
printf '// Tool count: 1\nexport type ToolName = "alpha_tool";\n' > "$root/packages/mcp-types/src/tools.ts"
run "$root"
expect_output "SDK types may leave out a tool only a coach is offered" "the SDK types match the 1 tools of the public profile"
root="$(tree coach_only_in_types)"
public "$root" '["alpha_tool"]'
run "$root"
expect_output "SDK types naming a coach-only tool fail Check 3" "SDK type drift"

root="$(tree sdk_lists_unserved)"
served "$root" '{"tools":[{"name":"alpha_tool"}]}'
public "$root" '["alpha_tool"]'
run "$root"
expect_output "SDK types naming an unserved tool fail Check 3" "SDK type drift"
expect_absent "no clean tool list over SDK types naming an unserved tool" "MCP tool list: 2 tools match"

root="$(tree phantom_served)"
served "$root" '{"tools":[{"name":"alpha_tool"},{"name":"beta_tool"},{"name":"ghost_tool"}]}'
run "$root"
expect_output "a served tool src does not register fails Check 3" "serves tools src does not register"

root="$(tree no_fixture)"
rm "$root/crates/pierre-server/tests/fixtures/mcp_surface/tools_list_admin.json"
run "$root"
expect_output "a missing served fixture fails Check 3" "tools_list_admin.json not found"

root="$(tree bad_fixture)"
served "$root" 'not json'
run "$root"
expect_output "an unreadable served fixture fails Check 3 closed" "Could not read the tool names"
expect_absent "no clean tool list over an unreadable fixture" "MCP tool list: 2 tools match"

root="$(tree empty_fixture)"
served "$root" '{"tools":[]}'
run "$root"
expect_output "an empty served fixture fails Check 3 closed" "lists no tools"

# The public profile's names fail closed the same way: a fixture that cannot be
# read, or names nothing, never reads as SDK types in sync.
root="$(tree bad_public_fixture)"
public "$root" 'not json'
run "$root"
expect_output "an unreadable public fixture fails Check 3 closed" "Could not read the public profile's tool names"
expect_absent "no clean tool list over an unreadable public fixture" "MCP tool list: 2 tools match"

root="$(tree empty_public_fixture)"
public "$root" '[]'
run "$root"
expect_output "an empty public fixture fails Check 3 closed" "Could not read the public profile's tool names"
expect_absent "no clean tool list over an empty public fixture" "MCP tool list: 2 tools match"

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

# 4b. A schema built in a shape the scan reads no name from: an array of
#     tuples mapped through a helper, as get_segment_history once was. The
#     tool would read as declaring nothing, so the shared scan refuses it.
root="$(tree unread_schema)"
cat >> "$root/crates/demo/src/tools.rs" <<'RS'

fn string_property(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

impl McpTool for Epsilon {
    fn definition(&self) -> Tool {
        let properties: HashMap<String, PropertySchema> = [("from", "Start date.")]
            .into_iter()
            .map(|(name, description)| (name.to_owned(), string_property(description)))
            .collect();
        tool_definition("epsilon_tool", "Epsilon", object_schema(properties, None))
    }
}
RS
run "$root"
expect_output "an unread schema fails Check 3" "Tool scan incomplete"
expect_output "the refusal names the tool whose schema went unread" "tools.rs: epsilon_tool"
expect_absent "no clean tool list is reported over an unread schema" "tools match"

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

# 13. Check 4 reads every locale. A line presenting a load ratio as injury risk
# fails, the same line forbidding it passes, and a negation word from another
# language excuses nothing: "no" is Spanish, and in Portuguese means "in the".
framing() {
    local agent="$CM/prompts/agents/training/framing-agent"
    mkdir -p "$agent"
    printf '%s\n' "$1" > "$agent/$2.md"
    run_override "$(catalogue_tree "framing_$3")" "$CM"
    rm -rf "$agent"
}
expect_framing_hit() {
    framing "$2" "$3" "$4"
    expect_output "$1" "[ACWR presented as injury risk]"
}
expect_framing_clean() {
    framing "$2" "$3" "$4"
    expect_output "$1" "Contremaitre catalogues carry no retired ACWR/TSB framing"
}
expect_green_verdict() {
    framing "$2" "$3" "$4"
    expect_output "$1" "[red/green safety verdict]"
}
expect_green_verdict "Spanish green band fails" "Un ACWR de 1,1 está en la banda verde." es es_green
expect_green_verdict "German green zone fails" "Ein ACWR von 1,1 liegt im grünen Bereich." de de_green
expect_green_verdict "Portuguese green band fails" "Um ACWR de 1,1 está na faixa verde." pt pt_green
expect_green_verdict "French green band still fails" "Un ACWR de 1,1 est dans la bande verte." fr fr_green
expect_framing_hit "Spanish injury framing fails" "Un ACWR de 1,6 es un riesgo de lesión." es es_hit
expect_framing_hit "Spanish framing with a word between fails" "El ACWR alto supone riesgo alto de lesión." es es_gap
expect_framing_clean "Spanish negated framing passes" "Nunca presentes el ACWR como un riesgo de lesión." es es_clean
expect_framing_hit "German injury framing fails" "Ein ACWR über 1,5 bedeutet Verletzungsrisiko." de de_hit
expect_framing_clean "German negated framing passes" "Das ACWR ist niemals ein Verletzungsrisiko." de de_clean
expect_framing_hit "Portuguese injury framing fails" "Um ACWR de 1,6 indica risco de lesão." pt pt_hit
expect_framing_hit "Portuguese 'no' negates nothing" "O risco de lesão aparece no ACWR acima de 1,5." pt pt_no
expect_framing_clean "Portuguese negated framing passes" "O ACWR não é um risco de lesão." pt pt_clean
expect_framing_hit "a Spanish 'no' does not excuse English framing" "ACWR above 1.5 is an injury risk, no matter the base." en en_no
expect_framing_hit "French framing still fails" "Un ACWR élevé est un risque de blessure." fr fr_hit
expect_framing_clean "French negated framing still passes" "Ne présente jamais l'ACWR comme un risque de blessure." fr fr_clean
expect_framing_clean "an injury word with no load word passes" "La fuerza reduce el riesgo de lesión." es es_noload

# 14. Check 4 reads every string in the catalogue: a messaging string is the
# platform's own voice — the envelope notice under an edited session among them
# (carnet#1495) — and answers to the same framing as a use case.
catalogue_framing() { # <text> <name> — the text as messaging.notice in every locale
    local saved="$TMP/strings_saved_$2"
    mkdir -p "$saved"
    cp "$CM"/strings/*.json "$saved/"
    for l in "${LOCALES[@]}"; do
        printf '{"common": {"greet": "Hi {0}", "cancel": "Cancel {{name}}"}, "messaging": {"notice": "%s"}}\n' \
            "$1" > "$CM/strings/$l.json"
    done
    run_override "$(catalogue_tree "catalogue_framing_$2")" "$CM"
    cp "$saved"/*.json "$CM/strings/"
}
catalogue_framing "This load spike raises your injury risk." msg_hit
expect_output "a messaging string framing load as injury risk fails" "[ACWR presented as injury risk]"
expect_output "the hit names the messaging key it read" "strings/en.json:messaging.notice"
catalogue_framing "Saved as written, outside what the evidence supports." msg_clean
expect_output "a messaging string without the framing passes" "Contremaitre catalogues carry no retired ACWR/TSB framing"

# 14b. A strings/ directory holding no catalogue fails Check 4 closed: a scan
# that read no string would otherwise report the same green as a clean one.
root="$(catalogue_tree catalogue_framing_none)"
mkdir -p "$TMP/strings_saved_none"
mv "$CM"/strings/*.json "$TMP/strings_saved_none/"
run_override "$root" "$CM"
mv "$TMP/strings_saved_none"/*.json "$CM/strings/"
expect_output "a strings directory with no catalogue fails Check 4 closed" "SCAN-INCOMPLETE: no string read from $CM/strings/*.json"
expect_absent "no clean framing scan over an unread catalogue" "Contremaitre catalogues carry no retired ACWR/TSB framing"

echo ""
if [[ "$failures" -gt 0 ]]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all cases passed"
