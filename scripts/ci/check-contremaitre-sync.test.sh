#!/usr/bin/env bash
# ABOUTME: Fixture test for check-contremaitre-sync.sh Check 3 — the MCP tool list comes from the one
# ABOUTME: registry enumeration (tool_schema_properties.py) and inherits every premise that scan asserts
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Check 3 used to grep `tool_definition(` itself, beside the scan Check 9 and
# check-declared-parameters.sh attribute properties with: two enumerations of
# one registry, each with its own "scan incomplete" guard (carnet#583). A tree
# that broke the shared scan's premise then read as "all tools in sync" to
# Check 3, and the only gate that noticed was Check 9 — which skips whenever the
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

echo "==== check-contremaitre-sync.sh Check 3 fixture test ===="

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

echo ""
if [[ "$failures" -gt 0 ]]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all cases passed"
