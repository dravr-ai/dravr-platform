#!/usr/bin/env bash
# ABOUTME: Fixture test for check-activity-writers.sh — makes it fire on a planted writer or prune, pass and fail closed
# ABOUTME: A cache write, row delete or by-date DELETE outside the activity store's sink fails; tests, fixtures and the repository do not
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-activity-writers.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

SINK="crates/pierre-services/src/activity_cache_sink.rs"
CALLER="crates/pierre-tool-runtime/src/activity_fetch/write_through.rs"

# A tree with the repository defining the writes, the sink calling them, and
# a provider read going through the activity store.
scaffold() {
    rm -rf "$WORK/root"
    local db="$WORK/root/crates/pierre-database/src/repositories"
    mkdir -p "$db/activity_cache" "$(dirname "$WORK/root/$SINK")" \
        "$(dirname "$WORK/root/$CALLER")"
    cat > "$db/activity_cache.rs" <<'RS'
pub trait ActivityCacheRepository {
    async fn upsert_activities(&self, u: Uuid, t: &TenantId, p: &str, a: &[Activity]) -> AppResult<u64> {
        self.upsert_activities_synced_at(u, t, p, a, Utc::now()).await
    }
}
RS
    cat > "$db/activity_cache/sql.rs" <<'RS'
pub(crate) const DELETE_CACHED_ACTIVITY_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";
RS
    cat > "$WORK/root/$SINK" <<'RS'
async fn project_live(&self, rows: &[ActivityRecord]) {
    self.repos
        .activity_cache
        .upsert_activities_synced_at(user, &tenant, source, &activities, synced_at)
        .await;
}
async fn retract(&self, id: &ActivityId) {
    self.repos.activity_cache.delete_cached_activity(user, &tenant, key, id.as_str()).await;
    self.repos.activity_cache.delete_provider_activities(user, &tenant, key).await;
}
RS
    cat > "$WORK/root/$CALLER" <<'RS'
pub async fn write_through(runtime: &dyn ToolRuntime) {
    runtime.live_activities().ingest(user, tenant, provider, &rows, now).await;
}
RS
}

run() { # prints the exit status; output in $WORK/out
    local code=0
    bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
    echo "$code"
}

echo "check-activity-writers.test.sh"

scaffold
code="$(run)"
if [ "$code" -eq 0 ] && grep -q "written and deleted only by the activity store's sink (1 write site(s))" "$WORK/out"; then
    pass "writes and deletes only in the sink and the repository pass"
else
    fail "a clean tree should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# A planted violation appended to the provider read: name, snippet.
expect_fires() {
    local what="$1" snippet="$2"
    scaffold
    printf '%s\n' "$snippet" >> "$WORK/root/$CALLER"
    local code
    code="$(run)"
    if [ "$code" -eq 1 ] && grep -Eq "write_through.rs:([4-9]):" "$WORK/out"; then
        pass "$what fails and is named"
    else
        fail "$what should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}
expect_fires "a direct upsert_activities" \
    'async fn land(r: &R) { r.activity_cache.upsert_activities(u, &t, "strava", &rows).await; }'
expect_fires "a direct upsert_activities_synced_at" \
    'async fn land(r: &R) { r.activity_cache.upsert_activities_synced_at(u, &t, "strava", &rows, at).await; }'
expect_fires "a UFCS upsert" \
    'async fn land(r: &R) { ActivityCacheRepository::upsert_activities(&*r.cache, u, &t, "garmin", &rows).await; }'
expect_fires "an upsert spread over a method chain" \
    'async fn land(r: &R) {
    r.activity_cache
        .upsert_activities(u, &t, "strava", &rows)
        .await;
}'
expect_fires "a direct delete_cached_activity" \
    'async fn drop(r: &R) { r.activity_cache.delete_cached_activity(u, &t, "upload", id).await; }'
expect_fires "a window evicted with delete_cached_activities_between" \
    'async fn evict(r: &R) {
    r.activity_cache
        .delete_cached_activities_between(u, &t, "sciotte", start, end)
        .await;
}'
expect_fires "a UFCS delete_provider_activities" \
    'async fn purge(r: &R) { ActivityCacheRepository::delete_provider_activities(&*r.cache, u, &t, "sciotte_trainingpeaks").await; }'
expect_fires "a direct cache amend" \
    'async fn power(r: &R) { r.activity_cache.amend_activity(u, &t, "garmin", &ride).await; }'
expect_fires "a 180-day prune by start_date" \
    'const PRUNE_SQL: &str = "DELETE FROM cached_activities WHERE user_id = $1 AND start_date < $2";'
expect_fires "a lowercase prune by synced_at over several lines" \
    'const EXPIRE_SQL: &str = r"
    delete from cached_activities
    where tenant_id = $1
      and synced_at < $2";'

scaffold
cat >> "$WORK/root/crates/pierre-database/src/repositories/activity_cache/sql.rs" <<'RS'
pub(crate) const PRUNE_ACTIVITIES_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND start_date < $3";
RS
code="$(run)"
if [ "$code" -eq 1 ] && grep -q "activity_cache/sql.rs" "$WORK/out"; then
    pass "a by-date prune in the repository fails and is named"
else
    fail "a by-date prune in the repository should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat >> "$WORK/root/$CALLER" <<'RS'

#[cfg(test)]
mod tests {
    async fn seed(r: &R) {
        r.activity_cache.upsert_activities(u, &t, "strava", &rows).await;
        r.activity_cache.delete_cached_activity(u, &t, "strava", "1").await;
        sqlx::query("DELETE FROM cached_activities WHERE synced_at < $1");
    }
}
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a write or prune in a trailing #[cfg(test)] module is not production code"
else
    fail "an inline test module should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
mkdir -p "$WORK/root/crates/pierre-test-support/src"
printf '%s\n' 'pub async fn seed(r: &R) { r.activity_cache.upsert_activities(u, &t, "strava", &rows).await; }' \
    > "$WORK/root/crates/pierre-test-support/src/activities.rs"
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a test fixture in pierre-test-support may write the cache"
else
    fail "pierre-test-support should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
printf '%s\n' 'const DELETE_ONE_SQL: &str = "DELETE FROM cached_activities WHERE activity_id = $1";' \
    >> "$WORK/root/$CALLER"
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a delete by key, not by date, is not a prune"
else
    fail "a delete by key should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
rm "$WORK/root/$SINK"
code="$(run)"
if [ "$code" -eq 2 ] && grep -q "sink.rs is missing" "$WORK/out"; then
    pass "a moved sink fails closed (exit 2)"
else
    fail "a missing sink should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
printf '%s\n' 'async fn project_live() {}' > "$WORK/root/$SINK"
code="$(run)"
if [ "$code" -eq 2 ] && grep -q "writes no cached activity" "$WORK/out"; then
    pass "a sink that no longer writes fails closed (exit 2)"
else
    fail "a sink without a write should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
code=0
PROD_SRC_SCAN="$WORK/missing-scan.sh" bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
if [ "$code" -ne 0 ] && [ "$code" -ne 1 ]; then
    pass "a scan that cannot run fails closed (exit $code), never a pass"
else
    fail "a broken scan should not pass or read as a violation (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

if [ "$FAILURES" -ne 0 ]; then
    echo "check-activity-writers.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-activity-writers.test.sh: all passed"
