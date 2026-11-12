#!/usr/bin/env bash
# ABOUTME: Fixture test for check-history-walk-single-source.sh — makes each rule fire on a planted copy, pass and fail closed
# ABOUTME: A spawn, a history_import or history_keys symbol, a stray coverage type or a second order fails; wire values do not
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-history-walk-single-source.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

BUDGETED_READ="crates/pierre-tool-runtime/src/activity_fetch/budgeted_read.rs"
SOURCE_ORDER="crates/pierre-services/src/training_history_read/source_order.rs"
RESOLVER="crates/pierre-providers/src/backend_resolver.rs"
COVERAGE="crates/pierre-core/src/models/coverage.rs"
CALLER="crates/pierre-tool-runtime/src/activity_fetch/historical.rs"
SUITE="crates/pierre-server/tests/it/backfill_job_kinds_migration_test.rs"

# A tree holding the one source order, the one read plan, the one Coverage,
# a budgeted read that awaits its work, and a caller reading through them.
scaffold() {
    rm -rf "$WORK/root"
    local file
    for file in "$BUDGETED_READ" "$SOURCE_ORDER" "$RESOLVER" "$COVERAGE" "$CALLER" "$SUITE"; do
        mkdir -p "$(dirname "$WORK/root/$file")"
    done
    cat > "$WORK/root/$BUDGETED_READ" <<'RS'
pub async fn refresh_head_within_budget(runtime: &R) -> Outcome {
    match timeout(budget, read_head(runtime)).await {
        Ok(rows) => Outcome::Fresh(rows),
        Err(_) => enqueue_live_read(runtime).await,
    }
}
RS
    cat > "$WORK/root/$SOURCE_ORDER" <<'RS'
pub enum HistorySourceRole { Session, Grant, Pushed }
pub struct HistorySource { pub backend: String, pub role: HistorySourceRole, pub live: bool }
pub fn history_sources_for(provider: &str) -> Vec<(&str, HistorySourceRole)> { vec![] }
pub fn history_store_keys(backend: &str) -> Vec<&str> { vec![backend] }
pub async fn history_source_order(repos: &R, provider: &str) -> Vec<HistorySource> {
    vec![HistorySource { backend: provider.to_owned(), role: HistorySourceRole::Session, live: true }]
}
pub async fn walk_source(repos: &R, provider: &str) -> Option<HistorySource> { None }
RS
    cat > "$WORK/root/$RESOLVER" <<'RS'
pub struct ReadPlan { pub primary: String, pub fallback: Option<String> }
impl ReadPlan {
    fn alone(backend: &str) -> Self { Self { primary: backend.to_owned(), fallback: None } }
}
pub async fn read_plan(repos: &R, provider: &str) -> ReadPlan {
    ReadPlan { primary: provider.to_owned(), fallback: None }
}
RS
    cat > "$WORK/root/$COVERAGE" <<'RS'
pub enum Pending { HistoryWalk { months_done: u32 } }
pub struct CoverageWindow { pub total: usize }
pub struct Coverage { pub window: CoverageWindow, pub pending: Option<Pending> }
struct CoverageWire { window: CoverageWindow }
RS
    cat > "$WORK/root/$CALLER" <<'RS'
use pierre_services::training_history_read::history_store_keys;
pub const KEY_HOSTED_PICKER_TAG_HISTORY_IMPORTING: &str = "hosted.picker.tagHistoryImporting";
pub async fn depth(runtime: &R, provider: &str) -> bool {
    for key in history_store_keys(provider) {
        let _ = read_plan(runtime.repos(), key).await.primary;
    }
    true
}
async fn read_plan(plan: &TrainingPlanRead) -> AppResult<Option<Plan>> { plan.load().await }
RS
    cat > "$WORK/root/$SUITE" <<'RS'
const SEED: &str = "INSERT INTO activity_backfill_jobs (kind) VALUES ('history_import')";
const KIND: &str = "history_import";
#[test]
fn history_store_keys_serve_both_halves() {}
RS
}

run() { # prints the exit status; output in $WORK/out
    local code=0
    bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
    echo "$code"
}

echo "check-history-walk-single-source.test.sh"

scaffold
code="$(run)"
if [ "$code" -eq 0 ] && grep -q "one history source order and one live-read order" "$WORK/out"; then
    pass "one order of each, one Coverage, quoted wire values and a training-plan read_plan pass"
else
    fail "a clean tree should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# A planted line appended to `file`: name, file, snippet, the file the
# output must name.
expect_fires() {
    local what="$1" file="$2" snippet="$3" named="${4:-$2}"
    scaffold
    mkdir -p "$(dirname "$WORK/root/$file")"
    printf '%s\n' "$snippet" >> "$WORK/root/$file"
    local code
    code="$(run)"
    if [ "$code" -eq 1 ] && grep -qF "$named" "$WORK/out"; then
        pass "$what fails and is named"
    else
        fail "$what should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}

# 1. A detached task in the budgeted read.
expect_fires "a tokio::spawn in the budgeted read" "$BUDGETED_READ" \
    'fn detach(r: R) { tokio::spawn(async move { r.backfill().await }); }'
expect_fires "an imported spawn in the budgeted read" "$BUDGETED_READ" \
    'use tokio::task::spawn;'
expect_fires "a spawn_blocking in the budgeted read" "$BUDGETED_READ" \
    'fn parse(rows: Rows) { tokio::task::spawn_blocking(move || decode(rows)); }'

# 2. The retired history import.
expect_fires "a history_import module" "$CALLER" 'pub mod history_import;'
expect_fires "a HistoryImport variant" "$CALLER" \
    'pub enum BackfillJobKind { HistoryWalk, HistoryImport }'
expect_fires "a HISTORY_IMPORT constant" "$CALLER" \
    'pub const HISTORY_IMPORT_PAGE: usize = 200;'
expect_fires "a history_import call in a test suite" "$SUITE" \
    'async fn seed(r: &R) { pierre_tool_runtime::start_history_import(r).await; }'
expect_fires "a file named after the history import" \
    "crates/pierre-tool-runtime/src/activity_backfill/history_import.rs" \
    'pub fn run() {}'

# 3. A second table of a provider's history keys.
expect_fires "a history_keys function" "$RESOLVER" \
    'pub fn history_keys(backend: &str) -> Vec<&str> { vec![backend] }'
expect_fires "a history_keys call" "$CALLER" \
    'fn keys(p: &str) -> usize { backend_resolver::history_keys(p).len() }'

# 4. Coverage types.
expect_fires "a regrown HistoryCoverage" \
    "crates/pierre-tool-runtime/src/training_history_compute.rs" \
    'pub struct HistoryCoverage { pub complete: bool }'
expect_fires "a regrown StatsCoverage enum" \
    "crates/pierre-core/src/models/athlete.rs" \
    'pub enum StatsCoverage { Lifetime, Window { days: u32 } }'
expect_fires "a regrown VolumeCoverage alias" \
    "crates/pierre-tool-runtime/src/volume.rs" \
    'pub type VolumeCoverage = Coverage;'
expect_fires "a second Coverage in another crate" \
    "crates/pierre-services/src/training_history_read.rs" \
    'pub struct Coverage { pub complete: bool }'
expect_fires "a listed coverage name in a different file" \
    "crates/pierre-services/src/athlete_state_v2.rs" \
    'pub struct DossierCoverage { pub covered: usize }'

scaffold
mkdir -p "$WORK/root/crates/pierre-evals/src/athlete_data/period"
printf '%s\n' 'pub struct SourceCoverage { pub last_sync: Option<i64> }' \
    > "$WORK/root/crates/pierre-evals/src/athlete_data/period/coverage.rs"
mkdir -p "$WORK/root/crates/pierre-services/src"
printf '%s\n' 'pub enum CoverageReading { Covered, RunningOut }' \
    > "$WORK/root/crates/pierre-services/src/fortnight.rs"
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a listed coverage name in its own file passes"
else
    fail "SourceCoverage and CoverageReading in their files should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# 5. A second order.
expect_fires "a second history_sources_for" \
    "crates/pierre-tool-runtime/src/activity_backfill/history_walk.rs" \
    'fn history_sources_for(provider: &str) -> Vec<&str> { vec![provider] }'
expect_fires "a second HistorySource type" \
    "crates/pierre-server/src/tools/implementations/endurance_history.rs" \
    'pub struct HistorySource { pub provider: String }'
expect_fires "a second ReadPlan type" \
    "crates/pierre-routes-auth/src/card_backend.rs" \
    'pub struct ReadPlan { pub primary: String }'
expect_fires "a ReadPlan assembled outside the resolver" \
    "crates/pierre-routes-auth/src/card_backend.rs" \
    'fn grant_first(grant: &str) -> Option<ReadPlan> { Some(ReadPlan { primary: grant.to_owned(), fallback: None }) }'
expect_fires "a function returning a bare ReadPlan outside the resolver" \
    "crates/pierre-tool-runtime/src/activity_fetch.rs" \
    'async fn grant_first_plan(repos: &R, p: &str) -> ReadPlan { backend_resolver::read_plan(repos, p).await }'
expect_fires "a live_read_order beside read_plan" \
    "crates/pierre-providers/src/backend_resolver/live.rs" \
    'pub async fn live_read_order(repos: &R, p: &str) -> Vec<String> { vec![] }'
expect_fires "a history_source_order_v2" \
    "crates/pierre-services/src/personal_bests.rs" \
    'pub fn history_source_order_v2(p: &str) -> Vec<&str> { vec![p] }'

scaffold
cat >> "$WORK/root/$CALLER" <<'RS'

#[cfg(test)]
mod tests {
    fn plan_of(primary: &str) -> ReadPlan { ReadPlan { primary: primary.to_owned(), fallback: None } }
    #[test]
    fn store_keys_are_the_sources() {}
}
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a ReadPlan literal or an order-named test in a trailing #[cfg(test)] module passes"
else
    fail "an inline test module should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# Fail closed.
expect_exit_2() { # what file-to-break action(rm|empty) message
    local what="$1" file="$2" action="$3" message="$4"
    scaffold
    if [ "$action" = rm ]; then
        rm "$WORK/root/$file"
    else
        printf '%s\n' '// moved' > "$WORK/root/$file"
    fi
    local code
    code="$(run)"
    if [ "$code" -eq 2 ] && grep -qF "$message" "$WORK/out"; then
        pass "$what fails closed (exit 2)"
    else
        fail "$what should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}
expect_exit_2 "a moved budgeted read" "$BUDGETED_READ" rm "budgeted_read.rs is missing"
expect_exit_2 "a moved source order" "$SOURCE_ORDER" rm "source_order.rs is missing"
expect_exit_2 "a source order without its table" "$SOURCE_ORDER" empty "no longer defines history_source_order"
expect_exit_2 "a resolver without read_plan" "$RESOLVER" empty "no longer defines ReadPlan"
expect_exit_2 "a coverage module without Coverage" "$COVERAGE" empty "no longer defines Coverage"

scaffold
code=0
PROD_SRC_SCAN="$WORK/missing-scan.sh" bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
if [ "$code" -ne 0 ] && [ "$code" -ne 1 ]; then
    pass "a production scan that cannot run fails closed (exit $code), never a pass"
else
    fail "a broken scan should not pass or read as a violation (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# The tree this gate guards passes it.
code=0
bash "$CHECK" "$REPO_ROOT" >"$WORK/out" 2>&1 || code=$?
if [ "$code" -eq 0 ]; then
    pass "the repository's own tree passes"
else
    fail "the repository's tree should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

if [ "$FAILURES" -ne 0 ]; then
    echo "check-history-walk-single-source.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-history-walk-single-source.test.sh: all passed"
