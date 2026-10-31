#!/usr/bin/env bash
# ABOUTME: Fixture test for check-one-writer.sh (guard G1) — makes it fire, pass and fail closed
# ABOUTME: A second INSERT or UPDATE file, or a wellness write called outside the store, fails; soft deletes and tests do not
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-one-writer.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# A tree holding each table's one owning file, the wellness store calling
# the repository writes, and a caller going through the store.
scaffold() {
    rm -rf "$WORK/root"
    local db="$WORK/root/crates/pierre-database/src/repositories"
    mkdir -p "$db/activity_cache" "$WORK/root/crates/pierre-services/src/garmin_push"
    cat > "$db/health_persistence.rs" <<'RS'
const UPSERT_DATA_SOURCE_SQL: &str = "INSERT INTO data_sources (id) VALUES ($1)";
const UPSERT_SLEEP_SESSION_SQL: &str = "INSERT INTO sleep_sessions (id) VALUES ($1)";
const UPSERT_RECOVERY_METRICS_SQL: &str = "INSERT INTO recovery_metrics (id) VALUES ($1)";
const UPSERT_HEALTH_SNAPSHOT_SQL: &str = "INSERT INTO health_snapshots (id) VALUES ($1)";
const UPSERT_DAILY_ACTIVITY_SQL: &str = "INSERT INTO daily_activity (id) VALUES ($1)";
const SOFT_DELETE_RECOVERY_METRIC_SQL: &str =
    "UPDATE recovery_metrics SET deleted_at = $1 WHERE id = $2";
RS
    cat > "$db/activity_cache/sql.rs" <<'RS'
const UPSERT_CACHED_ACTIVITY_SQL: &str = "INSERT INTO cached_activities (id) VALUES ($1)";
const STORE_DETAIL_SQL: &str = r"
    UPDATE cached_activities
    SET detail_json = $1";
RS
    printf 'const TS_INSERT_POINT_SQL: &str = "INSERT INTO data_point_series (id) VALUES ($1)";\n' \
        > "$db/time_series.rs"
    printf 'const UPSERT_SQL: &str = "INSERT INTO activity_streams (id) VALUES ($1)";\n' \
        > "$db/activity_streams.rs"
    cat > "$WORK/root/crates/pierre-services/src/wellness_store.rs" <<'RS'
pub async fn sleep(repos: &Repos) {
    repos.sleep.upsert_sleep_session(&tenant, &session, now).await;
}
RS
    cat > "$WORK/root/crates/pierre-services/src/garmin_push/health.rs" <<'RS'
pub async fn land(repos: &Repos) {
    WellnessStore::new(repos).writes("push").sleep(tenant, &session, now).await;
}
RS
}

run() { # prints the exit status; output in $WORK/out
    local code=0
    bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
    echo "$code"
}

echo "check-one-writer.test.sh"

scaffold
code="$(run)"
if [ "$code" -eq 0 ] && grep -q "each written from one file" "$WORK/out"; then
    pass "one owning file per table, writes through the store, passes"
else
    fail "a clean tree should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# A second file writing a table: name, path under crates/, contents.
expect_second_writer() {
    local what="$1" path="$2" snippet="$3"
    scaffold
    mkdir -p "$(dirname "$WORK/root/$path")"
    printf '%s\n' "$snippet" > "$WORK/root/$path"
    local code
    code="$(run)"
    if [ "$code" -eq 1 ] && grep -q "$(basename "$path"):" "$WORK/out"; then
        pass "$what fails and is named"
    else
        fail "$what should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}
expect_second_writer "a second INSERT INTO sleep_sessions" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const UPSERT_PUSHED_SLEEP_SQL: &str = "INSERT INTO sleep_sessions (id) VALUES ($1)";'
expect_second_writer "a second recovery writer spelled without a space before the columns" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const MERGE_SQL: &str = "INSERT INTO recovery_metrics(id) VALUES ($1)";'
expect_second_writer "an UPDATE of cached_activities in another file" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const AMEND_PUSHED_ACTIVITY_SQL: &str = r"
    UPDATE cached_activities SET data_json = $1";'
expect_second_writer "a daily activity write outside its repository file" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const RAW: &str = "INSERT INTO daily_activity (id) VALUES ($1)";'
expect_second_writer "a body write outside the repository layer" \
    crates/pierre-server/src/services/sink.rs \
    'const RAW: &str = "INSERT INTO health_snapshots (id) VALUES ($1)";'
expect_second_writer "a lowercase insert" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const RAW: &str = "insert into sleep_sessions (id) VALUES ($1)";'
expect_second_writer "an INSERT with two spaces before INTO" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const RAW: &str = "INSERT  INTO recovery_metrics (id) VALUES ($1)";'
expect_second_writer "an UPDATE whose table sits on the next line" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const RAW: &str = r"
    UPDATE
        health_snapshots SET weight = $1";'
expect_second_writer "a SOFT_DELETE_* constant that also writes a reading" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const SOFT_DELETE_RECOVERY_SQL: &str = "UPDATE recovery_metrics SET deleted_at = $1, hrv_ms = $2 WHERE id = $3";'
expect_second_writer "a reading write placed after a SOFT_DELETE_* constant" \
    crates/pierre-database/src/repositories/pushed_data.rs \
    'const SOFT_DELETE_X_SQL: &str = "UPDATE recovery_metrics SET deleted_at = $1 WHERE id = $2";
async fn merge(pool: &Pool) { sqlx::query("UPDATE recovery_metrics SET hrv_ms = $1 WHERE id = $2"); }'

scaffold
printf 'const INSERT_SQL: &str = "INSERT INTO sleep_sessions_archive (id) VALUES ($1)";\n' \
    > "$WORK/root/crates/pierre-database/src/repositories/archive.rs"
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a table whose name only starts with a guarded one is not that table"
else
    fail "sleep_sessions_archive is not sleep_sessions (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat > "$WORK/root/crates/pierre-database/src/repositories/purge.rs" <<'RS'
const RETIRE_NIGHT_SQL: &str = "UPDATE sleep_sessions SET deleted_at = $1 \
                 WHERE id = $2";
async fn retire_day(pool: &Pool) {
    sqlx::query("update recovery_metrics set deleted_at = ? where id = ?");
}
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a soft delete, by its shape under any name, is not a second writer"
else
    fail "a soft delete should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# A repository wellness write called outside the store: name, snippet.
expect_call_fires() {
    local what="$1" snippet="$2"
    scaffold
    printf '%s\n' "$snippet" >> "$WORK/root/crates/pierre-services/src/garmin_push/health.rs"
    local code
    code="$(run)"
    if [ "$code" -eq 1 ] && grep -q "health.rs:4:" "$WORK/out"; then
        pass "$what outside the store fails"
    else
        fail "$what outside the store should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}
expect_call_fires "a recovery upsert" \
    'async fn merge(r: &R) { r.recovery.upsert_recovery_metrics(&t, &row, mode, at).await; }'
expect_call_fires "a UFCS body upsert" \
    'async fn merge(r: &R) { HealthSnapshotRepository::upsert_health_snapshot(&*r.h, &t, &row, mode, at).await; }'
expect_call_fires "a daily activity upsert" \
    'async fn land(r: &R) { r.daily_activity.upsert_daily_activity(&t, &day, mode, at).await; }'
expect_call_fires "a data-source upsert" \
    'async fn source(r: &R) { r.data_sources.upsert_data_source(&t, &s).await; }'
expect_call_fires "a score clear" \
    'async fn clear(r: &R) { r.sleep.clear_provider_sleep_scores(u, &t, "whoop").await; }'
expect_call_fires "a series batch insert" \
    'async fn land(r: &R) { r.time_series_points.insert_batch(&id, 1, points).await; }'
expect_call_fires "a single series point insert" \
    'async fn land(r: &R) { r.time_series_points.insert(&id, 1, point).await; }'

scaffold
cat >> "$WORK/root/crates/pierre-services/src/garmin_push/health.rs" <<'RS'
async fn land(repos: &Repos) {
    repos
        .time_series_points
        .insert_batch(&id, 1, points)
        .await;
}
RS
code="$(run)"
if [ "$code" -eq 1 ] && grep -q "health.rs:7:" "$WORK/out"; then
    pass "a series insert spread over a method chain outside the store fails"
else
    fail "a chained series insert outside the store should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat >> "$WORK/root/crates/pierre-database/src/repositories/time_series.rs" <<'RS'
async fn insert(&self, id: &str, point: DataPoint) { self.insert_batch(id, 1, vec![point]).await }
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "the series repository delegating to its own batch insert is not an outside caller"
else
    fail "a repository's own delegation should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat >> "$WORK/root/crates/pierre-services/src/garmin_push/health.rs" <<'RS'

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn seeds() { repos.recovery.upsert_recovery_metrics(&t, &row, mode, at).await; }
}
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a write inside a trailing test module is not production code"
else
    fail "a test-module write should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
rm "$WORK/root/crates/pierre-services/src/wellness_store.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a missing wellness store fails closed"
else
    fail "a missing store should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
printf 'pub fn nothing() {}\n' > "$WORK/root/crates/pierre-services/src/wellness_store.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a store that no longer writes fails closed"
else
    fail "an empty store should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
rm "$WORK/root/crates/pierre-database/src/repositories/time_series.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a missing owning file fails closed"
else
    fail "a missing owner should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

if [ "$FAILURES" -gt 0 ]; then
    echo "check-one-writer.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-one-writer.test.sh: all passed"
