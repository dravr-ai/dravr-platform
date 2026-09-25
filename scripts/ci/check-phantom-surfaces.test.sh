#!/usr/bin/env bash
# ABOUTME: Fixture test for check-phantom-surfaces.sh's route gate and web service scan — what the old scans missed
# ABOUTME: Pins multi-line, {param} and .nest routes, the stock/diff contract, the marker, uncalled web methods, shared hooks, fail-closed scans
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The route half of Tier 1c read `.route("/api/…"` one line at a time and cut
# every path at its first `{param}`. rustfmt puts most registrations on three
# lines, so 160 `/api` paths were never read, and `/api/agents/{id}/fork` shrank
# to `/api/agents`, which a client mentions — dead routes passed green (carnet#583).
# Every case below that expects exit 1 exits 0 against that scan.
#
# The gate anchors on ITS OWN location (`$SCRIPT_DIR/../..`), so each fixture
# hosts copies of the gate, the route scanner and the base resolver at
# <root>/scripts/ci/; run against the real tree, the assertions would mean
# nothing. No fixture has an origin/main, so a run without an argument resolves
# its base to HEAD~1, exactly as gate-base-ref.sh does on a fresh clone.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-phantom-surfaces.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
OUT="$TMP/gate.out"

git_q() { git -c user.email=t@t -c user.name=t -c commit.gpgsign=false "$@"; }

# tree <name> — a minimal workspace every scan of the gate can read: one trait
# with an implementor, one deserialized config field, one api-client method
# both clients call, one web service method a component calls, and one /api
# route the api-client names.
tree() {
    local root="$TMP/$1"
    mkdir -p "$root/scripts/ci" "$root/crates/demo/src" \
             "$root/packages/api-client/src/domains" "$root/frontend/src/services/api" "$root/frontend-mobile/src"
    cp "$UNDER_TEST" "$root/scripts/ci/check-phantom-surfaces.sh"
    cp "$SCRIPT_DIR/backend-routes.py" "$root/scripts/ci/backend-routes.py"
    cp "$SCRIPT_DIR/gate-base-ref.sh" "$root/scripts/ci/gate-base-ref.sh"
    cat > "$root/crates/demo/src/lib.rs" <<'RS'
use serde::Deserialize;

pub mod routes;

pub trait Greeter {
    fn greet(&self);
}

pub struct Loud;

impl Greeter for Loud {
    fn greet(&self) {}
}

#[derive(Deserialize)]
pub struct DemoConfig {
    pub level: u8,
}

pub fn parse(text: &str) -> Option<DemoConfig> {
    serde_yaml::from_str(text).ok()
}
RS
    cat > "$root/packages/api-client/src/domains/demo.ts" <<'TS'
export const createDemoApi = (axios: { get: (url: string) => unknown }) => ({
    async listThings() {
      return axios.get('/api/things');
    },
});
TS
    printf 'export const load = (api: any) => api.demo.listThings();\n' > "$root/frontend/src/App.tsx"
    printf 'export const load = (api: any) => api.demo.listThings();\n' > "$root/frontend-mobile/src/App.tsx"
    web_service "$root" ""
    printf "import { gadgetsApi } from './services/api/gadgets';\nexport const show = () => gadgetsApi.loadGadgets();\n" \
        > "$root/frontend/src/Gadgets.tsx"
    routes "$root" ""
    ( cd "$root" && git init -q . )
    echo "$root"
}

# web_service <root> <extra methods> — rewrites the web-only service with the
# base `loadGadgets` method plus whatever the case adds.
web_service() {
    {
        printf 'export const gadgetsApi = {\n'
        printf '  async loadGadgets() {\n    return fetch(\x27/gadgets\x27);\n  },\n'
        printf '%s' "$2"
        printf '};\n'
    } > "$1/frontend/src/services/api/gadgets.ts"
}

# routes <root> <extra registrations> — rewrites the router with the base
# `/api/things` route plus whatever the case adds.
routes() {
    {
        printf 'use axum::Router;\nuse axum::routing::{get, post};\n\n'
        printf 'pub fn build_router() -> Router {\n    Router::new()\n'
        printf '        .route("/api/things", get(list))\n'
        printf '%s' "$2"
        printf '}\n'
    } > "$1/crates/demo/src/routes.rs"
}

commit() { ( cd "$1" && git add -A && git_q commit -q -m "$2" ); }

# expect <label> <root> <want-exit> [base-ref] — an empty base runs the gate
# with no argument at all.
expect() {
    local label="$1" root="$2" want="$3" base="${4:-}" got=0
    if [[ -n "$base" ]]; then
        ( cd "$root" && env -u GATE_BASE_REF bash scripts/ci/check-phantom-surfaces.sh "$base" ) >"$OUT" 2>&1 || got=$?
    else
        ( cd "$root" && env -u GATE_BASE_REF bash scripts/ci/check-phantom-surfaces.sh ) >"$OUT" 2>&1 || got=$?
    fi
    if [[ "$got" == "$want" ]]; then
        pass "$label (exit $got)"
    else
        fail "$label — wanted exit $want, got $got"
        sed 's/^/      /' "$OUT"
    fi
}

expect_output() { # $1 = label, $2 = literal the last run must print
    if grep -qF -- "$2" "$OUT"; then pass "$1"; else
        fail "$1 (output is missing: $2)"
        sed 's/^/      /' "$OUT"
    fi
}

MULTILINE_WIDGETS='        .route(
            "/api/widgets",
            post(create_widget),
        )
'

echo "==== check-phantom-surfaces.sh route-gate fixture test ===="

# ---------------------------------------------------------------------------
# 1. A route rustfmt split across lines, which no client calls
# ---------------------------------------------------------------------------
root="$(tree multiline)"
commit "$root" base
routes "$root" "$MULTILINE_WIDGETS"
commit "$root" "add a split registration"
expect "a new multi-line route no client calls fails" "$root" 1 HEAD~1
expect_output "the refusal names the multi-line route" "New /api/ route no client mentions: /api/widgets"

# ---------------------------------------------------------------------------
# 2. A {param} route whose static prefix a client mentions
# ---------------------------------------------------------------------------
# The old scan cut this to `/api/things`, which demo.ts names, and passed.
root="$(tree param)"
commit "$root" base
routes "$root" '        .route("/api/things/{id}/fork", post(fork))
'
commit "$root" "add a parameterised route"
expect "a new {param} route is matched on its whole template" "$root" 1 HEAD~1
expect_output "the refusal names the full template" "New /api/ route no client mentions: /api/things/{id}/fork"

# ...and the same route passes once a client builds that path.
cat >> "$root/packages/api-client/src/domains/demo.ts" <<'TS'
export const forkThing = (axios: { post: (url: string) => unknown }, id: string) =>
  axios.post(`/api/things/${id}/fork`);
TS
commit "$root" "call it"
expect "a {param} route a client interpolates passes" "$root" 0 HEAD~2

# ---------------------------------------------------------------------------
# 3. A router mounted under a .nest prefix through a let binding
# ---------------------------------------------------------------------------
# The binding wraps the builder in a closure whose body has its own `;`, the
# shape /api/admin/config is mounted with — a `[^;]*;` let parser stopped there.
root="$(tree nested)"
commit "$root" base
cat > "$root/crates/demo/src/admin.rs" <<'RS'
use axum::Router;
use axum::routing::get;

pub fn admin_router(level: u8) -> Router {
    Router::new().route("/gadgets", get(move || async move { level.to_string() }))
}
RS
cat >> "$root/crates/demo/src/lib.rs" <<'RS'

pub mod admin;

pub fn mount(app: axum::Router, level: Option<u8>) -> axum::Router {
    let admin_routes = level.map_or_else(
        || {
            tracing::warn!("admin API disabled");
            axum::Router::new()
        },
        |l| admin::admin_router(l),
    );
    app.nest("/api/admin", admin_routes)
}
RS
commit "$root" "mount an admin router"
expect "a new nested route no client calls fails" "$root" 1 HEAD~1
expect_output "the refusal carries the mount prefix" "New /api/ route no client mentions: /api/admin/gadgets"
prefixes="$(cd "$root" && python3 scripts/ci/backend-routes.py prefixes .)"
if grep -qx api <<< "$prefixes" && ! grep -qx gadgets <<< "$prefixes"; then
    pass "the nginx prefix view resolves the mount too (api, not gadgets)"
else
    fail "the nginx prefix view did not resolve the mount: $(tr '\n' ' ' <<< "$prefixes")"
fi

# ---------------------------------------------------------------------------
# 4. The contract: the stock is reported, never gated
# ---------------------------------------------------------------------------
# /api/legacy has no client at the base. Reflowing it onto three lines adds
# lines, not a route, so the push passes and the stock still lists it.
root="$(tree stock)"
routes "$root" '        .route("/api/legacy", get(legacy))
'
commit "$root" base
routes "$root" '        .route(
            "/api/legacy",
            get(legacy),
        )
'
commit "$root" "rustfmt reflow"
expect "reflowing a standing orphan does not fail the push" "$root" 0 HEAD~1
expect_output "the standing orphan is still reported" "   /api/legacy"

# ---------------------------------------------------------------------------
# 5. A registered gap, and a marker that names a different route
# ---------------------------------------------------------------------------
root="$(tree marker)"
commit "$root" base
routes "$root" '        // LIMITATION(registre#9): /api/widgets has no client
'"$MULTILINE_WIDGETS"
commit "$root" "add a registered route"
expect "a new route whose declaration names it in a LIMITATION marker passes" "$root" 0 HEAD~1
expect_output "the registered gap stays visible with its issue" "Registered gap (registre#9): /api/widgets has no client"

routes "$root" '        // LIMITATION(registre#9): /api/widgets has no client
'"$MULTILINE_WIDGETS"'        .route(
            "/api/widgets/extra",
            get(extra),
        )
'
commit "$root" "add a longer route under the same marker"
expect "a marker for /api/widgets does not register /api/widgets/extra" "$root" 1 HEAD~1
expect_output "the unregistered route is named" "New /api/ route no client mentions: /api/widgets/extra"

# ---------------------------------------------------------------------------
# 6. No argument still gates
# ---------------------------------------------------------------------------
# An empty base used to mean "stock only", which is what every
# workflow_dispatch run passed. The shared rule resolves it to HEAD~1 here.
root="$(tree noarg)"
commit "$root" base
routes "$root" "$MULTILINE_WIDGETS"
commit "$root" "add a split registration"
expect "with no argument the gate diffs against the shared base and fails" "$root" 1

# ---------------------------------------------------------------------------
# 7. Fail closed: a scan that cannot stand behind its list
# ---------------------------------------------------------------------------
root="$(tree unresolved)"
commit "$root" base
cat >> "$root/crates/demo/src/lib.rs" <<'RS'

pub fn mount(app: axum::Router, mystery: axum::Router) -> axum::Router {
    app.nest("/api/admin", mystery)
}
RS
commit "$root" "nest something the scan cannot resolve"
expect "an unresolvable .nest fails rather than dropping its routes" "$root" 1 HEAD~1
expect_output "the refusal names the mount" "resolves to no function that declares a route"

root="$(tree computed)"
commit "$root" base
routes "$root" '        .route(WIDGETS_PATH, get(widgets))
'
commit "$root" "register a computed path"
expect "a computed .route( path fails rather than going unread" "$root" 1 HEAD~1
expect_output "the refusal names the unreadable registration" ".route( without a string-literal path"

# ---------------------------------------------------------------------------
# 8. Web-only service methods (frontend/src/services/api)
# ---------------------------------------------------------------------------
# Scan 2 reads only the api-client, so a web service method nothing calls
# passed, and the route it named read as consumed (carnet#581: 34 of them).
root="$(tree web)"
commit "$root" base
expect "a web service method a component calls passes" "$root" 0 HEAD~1
expect_output "the scan reports the method it read" "Web services: all 1 methods in frontend/src/services/api have a production caller."

web_service "$root" '  async loadGadget(id: string) {
    return fetch(`/gadgets/${id}`);
  },
'
commit "$root" "add a method no component calls"
expect "a web service method no component calls fails" "$root" 1 HEAD~1
expect_output "the refusal names the uncalled method" "   loadGadget  (frontend/src/services/api/gadgets.ts:"

# A test that exercises the method is not a caller.
mkdir -p "$root/frontend/src/__tests__"
printf "import { gadgetsApi } from '../services/api/gadgets';\ngadgetsApi.loadGadget('g1');\n" \
    > "$root/frontend/src/__tests__/gadgets.test.ts"
commit "$root" "cover it with a test only"
expect "a caller in a test file does not count" "$root" 1 HEAD~1

# Whole-tree: deleting the component that held the last call leaves the method
# dead even though the diff adds no method at all.
root="$(tree webdelete)"
commit "$root" base
rm "$root/frontend/src/Gadgets.tsx"
commit "$root" "delete the only caller"
expect "deleting the file with the last call fails" "$root" 1 HEAD~1
expect_output "the refusal names the orphaned method" "   loadGadgets  (frontend/src/services/api/gadgets.ts:"

root="$(tree webgone)"
commit "$root" base
rm -r "$root/frontend/src/services"
commit "$root" "move the services somewhere the scan does not look"
expect "a missing services directory fails closed" "$root" 1 HEAD~1
expect_output "the refusal says the scan is stale" "frontend/src/services/api not found — this scan is stale."

# ---------------------------------------------------------------------------
# 9. A method reached through a shared hook both clients bind
# ---------------------------------------------------------------------------
# The React Query hooks live in packages/ui-logic as factories each client
# binds to its own API instance, so the call sits there and in neither app.
root="$(tree sharedhook)"
commit "$root" base
cat > "$root/packages/api-client/src/domains/demo.ts" <<'TS'
export const createDemoApi = (axios: { get: (url: string) => unknown }) => ({
    async listThings() {
      return axios.get('/api/things');
    },
    async countThings() {
      return axios.get('/api/things');
    },
});
TS
mkdir -p "$root/packages/ui-logic/src"
printf 'export const createThingHooks = (api: any) => ({ useCount: () => api.countThings() });\n' \
    > "$root/packages/ui-logic/src/thingHooks.ts"
commit "$root" "add a method only a shared hook calls"
expect "a method only a shared ui-logic hook calls passes" "$root" 0 HEAD~1
expect_output "the scan counts it as called" "api-client: all 2 domain methods have a production caller."

echo ""
if [[ "$failures" -gt 0 ]]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all cases passed"
