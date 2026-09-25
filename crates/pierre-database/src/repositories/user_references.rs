// ABOUTME: The rows that reference a user: those that block deleting the account, read in one statement,
// ABOUTME: and those the delete clears itself where no foreign key cascades, proven gone before it commits

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What stands between an operator and a user delete.
//!
//! Most rows referencing `users(id)` cascade or set NULL, and a delete takes
//! them along. The ones in [`DELETION_BLOCKERS_SQL`] do neither: deleting the
//! user while one exists fails in the database. They are group ownership and
//! coaching, invites, and operator or audit attribution: things an operator
//! reassigns deliberately rather than things a delete should decide. The
//! statement names each one so the refusal can say what to reassign.
//!
//! `coaching_group_members` is deliberately absent: a plain membership is the
//! user's own row and the delete clears it (see below), where a group they own
//! is somebody else's room.
//!
//! Three more refuse the delete although a foreign key would let it through,
//! because it would take something from other people or leave it running
//! elsewhere: a tenant the user alone owns while others belong to it (every
//! tenant read joins the owner's membership, so the tenant would vanish for
//! them), an agent the user authored that other users rely on (its rows
//! cascade with its author, or fail on another user's conversation or group
//! after the grants were already withdrawn), and a billing subscription the
//! provider may still charge (the row going cancels nothing there).
//!
//! The rest of what a user owns is keyed by a `user_id` column. Where that
//! column's foreign key cascades on both engines the account delete takes the
//! row along. Where it does not on one engine or the other — most were created
//! as `TEXT` beside a `uuid` `users.id` on Postgres, where no foreign key can
//! bind them, and `SQLite` leaves several unbound too — the row would outlive
//! the account. Each engine's [`UserPurge`] names every such table on that
//! engine ([`SQLITE_USER_PURGE`], [`POSTGRES_USER_PURGE`]), and a delete clears
//! each of them in the same transaction as the account row, then reads them
//! again and refuses to commit while any row survives. A table only the
//! `PostgreSQL` schema carries is named apart
//! ([`POSTGRES_ONLY_USER_OWNED_TABLES`]) and cleared and read back on that
//! engine alone.
//!
//! A row can also be the user's through a parent the account delete cascades
//! to, under a foreign key that does not cascade from that parent: a
//! messaging session goes with its user, while its messages (and their
//! delivery receipts and outbound retries) reference it with no `ON DELETE`,
//! so the cascade itself would fail. And a key can be the user's in effect
//! while another row owns it: an A2A client's API key belongs to the client's
//! system user, so it outlives the client that carried it. The list names
//! those tables too, each with the predicate that reaches the user's rows, in
//! an order that clears a referencing row before the row it references.
//!
//! Every id column compared here is `uuid` on Postgres and `TEXT` on `SQLite`,
//! matching the codec the shell binds `$1` through, and `$1` is bound once on
//! both engines however often it appears. `users.approved_by` carries no
//! foreign key on `SQLite`, and is listed on both so the two engines refuse the
//! same deletes. `SQLite`'s `user_llm_credentials_audit.changed_by` has no
//! Postgres counterpart and no writer; a violation on it still reaches the
//! caller as the conflict [`delete_user_error`] builds.

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{UserReference, UserReferenceKind};
use sqlx::error::DatabaseError;
use uuid::Uuid;

/// The rows of a table a user owns by its own `user_id` column: the predicate
/// every [`UserPurge`] entry takes unless it names one.
macro_rules! owned_rows {
    () => {
        "CAST(user_id AS TEXT) = $1"
    };
    ($predicate:literal) => {
        $predicate
    };
}

/// The tables one engine's user delete clears itself, and the statements it
/// runs over them, in the order it clears them.
#[derive(Debug, Clone, Copy)]
pub struct UserPurge {
    /// Every table cleared, in order.
    pub tables: &'static [&'static str],
    /// One `DELETE` per entry of [`Self::tables`], in its order.
    pub statements: &'static [&'static str],
    /// The listed tables that still hold a row for the user, with the count,
    /// read inside the delete's transaction before it commits.
    pub surviving: &'static str,
}

/// One engine's [`UserPurge`], built from `table [where predicate]` entries.
macro_rules! user_purge {
    ($($table:literal $(where $predicate:literal)?),+) => {
        UserPurge {
            tables: &[$($table),+],
            statements: &[$(
                concat!("DELETE FROM ", $table, " WHERE ", owned_rows!($($predicate)?))
            ),+],
            surviving: concat!(
                "SELECT tbl, n FROM (SELECT '' AS tbl, 0 AS n WHERE 1 = 0",
                $(
                    " UNION ALL SELECT '", $table, "', COUNT(*) FROM ", $table,
                    " WHERE ", owned_rows!($($predicate)?)
                ),+,
                ") survivors WHERE n > 0"
            ),
        }
    };
}

/// Declares the tables a user delete clears itself and the statements it runs
/// over them, from one list, so the delete, the survivor check and the list a
/// schema test holds against the live catalog cannot name different tables.
///
/// `both` holds the tables both schemas carry; `postgres_only` the ones only
/// the `PostgreSQL` migrations create, which the `SQLite` delete must not name
/// (its statement would fail on a missing table). Each engine's
/// [`UserPurge`] is derived here: `SQLite`'s from `both`, `PostgreSQL`'s from
/// `both` followed by `postgres_only`.
///
/// An entry is a table, optionally followed by `where` and the predicate that
/// reaches the user's rows in it; without one the rows are the table's own
/// `user_id`'s ([`owned_rows`]). `user_id`'s type differs per table and per
/// engine (`uuid`, `TEXT`, `VARCHAR`), so each comparison casts the column to
/// text and binds the id as its hyphenated text, which every one of those
/// types renders it as.
macro_rules! user_owned_tables {
    (
        both: [$($table:literal $(where $predicate:literal)?),+ $(,)?],
        postgres_only: [$($pg_table:literal $(where $pg_predicate:literal)?),+ $(,)?] $(,)?
    ) => {
        /// The tables only the `PostgreSQL` schema carries that keep a user's
        /// rows past the account delete, cleared after the tables both engines
        /// carry ([`SQLITE_USER_PURGE`]'s) on that engine alone.
        ///
        /// `agents_orphaned` is the quarantine a `PostgreSQL`-only migration
        /// moved agents whose `tenant_id` was not a tenant into, with the
        /// columns of `agents` and none of its constraints: the live
        /// `agents.user_id` cascades from the account, its quarantined copies
        /// keep the author's id with no foreign key to take them along.
        pub const POSTGRES_ONLY_USER_OWNED_TABLES: &[&str] = &[$($pg_table),+];

        /// What the `SQLite` user delete clears: every table both engines'
        /// user delete clears itself, in the order it clears them.
        ///
        /// Each holds a user's own rows without a foreign key that cascades the
        /// account delete to them on both engines: by its `user_id`, or through
        /// a row the user owns (a messaging session's messages, an A2A
        /// client's API key). A table whose rows reference another listed
        /// table's, or a row the account delete cascades to, without
        /// cascading comes before it (`sleep_sessions`, `recovery_metrics` and
        /// `health_snapshots` before `data_sources`; the messaging rows before
        /// the account row takes their sessions; the A2A keys before
        /// `a2a_clients` takes the link naming them). `chat_conversations`
        /// cascades from the account, and is cleared first anyway: a
        /// conversation with an agent the user authored references that agent
        /// with no `ON DELETE`, and on Postgres the agent's cascade can run
        /// before the conversation's and find it still there.
        ///
        /// Each is the user's own data: memberships, provider tokens and
        /// connections, health and activity rows, plans, facts, notifications,
        /// messaging history, device tokens, OAuth server grants, A2A client
        /// keys, usage records. A row attributing an act to the user in
        /// someone else's record is not here: those that do not cascade block
        /// the delete instead ([`DELETION_BLOCKERS_SQL`]).
        pub const SQLITE_USER_PURGE: UserPurge =
            user_purge!($($table $(where $predicate)?),+);

        /// What the `PostgreSQL` user delete clears: [`SQLITE_USER_PURGE`]'s
        /// tables, then [`POSTGRES_ONLY_USER_OWNED_TABLES`].
        pub const POSTGRES_USER_PURGE: UserPurge = user_purge!(
            $($table $(where $predicate)?),+,
            $($pg_table $(where $pg_predicate)?),+
        );
    };
}

user_owned_tables!(
    both: [
    "coaching_group_members",
    "user_oauth_tokens",
    "provider_connections",
    "oauth_client_states",
    "oauth_client_grants",
    "oauth2_auth_codes",
    "oauth2_refresh_tokens",
    "oauth2_states",
    "api_keys" where "id IN (SELECT l.api_key_id FROM a2a_client_api_keys l JOIN a2a_clients c ON c.client_id = l.client_id WHERE CAST(c.user_id AS TEXT) = $1)",
    "a2a_clients",
    "device_tokens",
    "notifications",
    "notification_preferences",
    "scheduled_notifications",
    "messaging_outbound_queue" where "CAST(user_id AS TEXT) = $1 OR message_id IN (SELECT m.id FROM messaging_messages m JOIN messaging_sessions s ON s.id = m.session_id WHERE CAST(s.user_id AS TEXT) = $1)",
    "messaging_delivery_receipts" where "message_id IN (SELECT m.id FROM messaging_messages m JOIN messaging_sessions s ON s.id = m.session_id WHERE CAST(s.user_id AS TEXT) = $1)",
    "messaging_messages" where "session_id IN (SELECT s.id FROM messaging_sessions s WHERE CAST(s.user_id AS TEXT) = $1)",
    "messaging_resumable_turns",
    "chat_conversations",
    "short_links",
    "mcp_tasks",
    "guardian_pending_actions",
    "agent_followups",
    "agent_notes",
    "agent_sessions",
    "athlete_commitments",
    "chat_message_feedback",
    "claim_verdicts",
    "coaching_playbooks",
    "pending_advice",
    "user_facts",
    "user_onboarding",
    "user_physiological_profiles",
    "user_llm_credentials_audit",
    "fitness_configurations",
    "training_plan_weeks",
    "training_plans",
    "training_history",
    "prescribed_workouts",
    "workout_templates",
    "route_summaries",
    "cached_activities",
    "activity_route_tracks",
    "activity_backfill_coverage",
    "activity_backfill_jobs",
    "activity_fetch_freshness",
    "backfill_push_log",
    "sleep_sessions",
    "recovery_metrics",
    "health_snapshots",
    "data_sources",
    "sync_state",
    "llm_usage",
    "usage_counters",
    "subscriptions",
    ],
    postgres_only: ["agents_orphaned"],
);

/// Clear a user completely inside one transaction: the user's rows in every
/// table the engine's [`UserPurge`] (`$purge`) names, then the account row
/// (whose foreign keys cascade to the rest), then prove nothing listed
/// survived before committing. Evaluates to `AppResult<UserDeletion>`; any
/// failure drops the transaction, rolling every statement back. A
/// foreign-key violation anywhere is the conflict [`delete_user_error`]
/// builds.
macro_rules! delete_user_completely {
    ($repo:expr, $ids:ident, $user_id:expr, $purge:ident) => {{
        let user_id: Uuid = $user_id;
        let owner = user_id.to_string();
        let tx = $repo
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("Failed to begin the user delete: {e}")))?;
        let mut guard = TransactionGuard::new(tx);
        let mut rows_removed = BTreeMap::new();
        for (table, statement) in $purge.tables.iter().zip($purge.statements) {
            let removed = sqlx::query(statement)
                .bind(&owner)
                .execute(guard.executor()?)
                .await
                .map_err(|e| delete_user_error(user_id, &e))?
                .rows_affected();
            if removed > 0 {
                rows_removed.insert((*table).to_owned(), removed);
            }
        }
        let deleted = sqlx::query(DELETE_USER_SQL)
            .bind($ids::bind(user_id))
            .execute(guard.executor()?)
            .await
            .map_err(|e| delete_user_error(user_id, &e))?;
        if deleted.rows_affected() == 0 {
            return Err(AppError::not_found(format!("User {user_id} not found")));
        }
        let survivors = sqlx::query($purge.surviving)
            .bind(&owner)
            .fetch_all(guard.executor()?)
            .await
            .map_err(|e| AppError::database(format!("Failed to verify the user delete: {e}")))?;
        if !survivors.is_empty() {
            let tables = survivors
                .iter()
                .map(|row| {
                    row.try_get::<String, _>("tbl").map_err(|e| {
                        AppError::database(format!("Failed to read a surviving table: {e}"))
                    })
                })
                .collect::<AppResult<Vec<String>>>()?;
            return Err(AppError::internal(format!(
                "User {user_id} delete left rows behind in {}; nothing was committed",
                tables.join(", ")
            )));
        }
        guard.commit().await?;
        Ok(UserDeletion { rows_removed })
    }};
}
pub(crate) use delete_user_completely;

/// Every reference onto one user the delete refuses over, as `(kind, detail)`
/// rows: the non-cascading foreign keys, and what the delete would take from
/// other people or leave live elsewhere.
///
/// A row the delete takes along anyway is excluded: an override or LLM
/// credential scoped to the user themselves cascades on `user_id`, and a
/// self-approval disappears with the user's own row. A tenant blocks only while
/// the user is its sole owner and someone else belongs to it; one the user is
/// alone in goes with its last member's row. An agent blocks while it is shared
/// beyond its author (tenant or global visibility) or another user's
/// conversation, a group, or another user's assignment is bound to it; a
/// private agent only its author used goes with them. A subscription blocks in
/// every status the provider may still bill in; a `canceled` or
/// `incomplete_expired` one is the user's own record and the delete clears it.
/// The `kind` literals are [`UserReferenceKind::as_str`].
pub const DELETION_BLOCKERS_SQL: &str = r"
            SELECT 'owns_coaching_group' AS kind, g.name AS detail
            FROM coaching_groups g
            WHERE g.owner_id = $1
            UNION ALL
            SELECT 'coaches_coaching_group', g.name
            FROM coaching_groups g
            WHERE g.coach_user_id = $1
            UNION ALL
            SELECT 'created_group_invite', g.name
            FROM group_invites i
            JOIN coaching_groups g ON g.id = i.group_id
            WHERE i.created_by = $1
            UNION ALL
            SELECT 'admin_config_override', o.category || '.' || o.config_key
            FROM admin_config_overrides o
            WHERE o.created_by = $1
              AND (o.user_id IS NULL OR o.user_id <> $1)
            UNION ALL
            SELECT 'admin_config_audit', a.category || '.' || a.config_key
            FROM admin_config_audit a
            WHERE a.admin_user_id = $1
            UNION ALL
            SELECT 'tenant_oauth_credentials',
                   c.provider || ' in tenant ' || CAST(c.tenant_id AS TEXT)
            FROM tenant_oauth_credentials c
            WHERE c.configured_by = $1
            UNION ALL
            SELECT 'llm_credentials', l.provider || ' in tenant ' || l.tenant_id
            FROM user_llm_credentials l
            WHERE l.created_by = $1
              AND (l.user_id IS NULL OR l.user_id <> $1)
            UNION ALL
            SELECT 'approved_user', u.email
            FROM users u
            WHERE u.approved_by = $1
              AND u.id <> $1
            UNION ALL
            SELECT 'owns_tenant', t.name
            FROM tenant_users tu
            JOIN tenants t ON t.id = tu.tenant_id
            WHERE tu.user_id = $1 AND tu.role = 'owner'
              AND EXISTS (
                  SELECT 1 FROM tenant_users m
                  WHERE m.tenant_id = tu.tenant_id AND m.user_id <> $1)
              AND NOT EXISTS (
                  SELECT 1 FROM tenant_users o
                  WHERE o.tenant_id = tu.tenant_id AND o.role = 'owner'
                    AND o.user_id <> $1)
            UNION ALL
            SELECT 'authored_agent', a.title
            FROM agents a
            WHERE a.user_id = $1
              AND (a.visibility <> 'private'
                   OR EXISTS (
                       SELECT 1 FROM chat_conversations c
                       WHERE c.agent_id = a.id AND c.user_id <> $1)
                   OR EXISTS (
                       SELECT 1 FROM coaching_groups g
                       WHERE g.agent_id = a.id)
                   OR EXISTS (
                       SELECT 1 FROM agent_assignments s
                       WHERE s.agent_id = a.id AND s.user_id <> $1))
            UNION ALL
            SELECT 'billing_subscription',
                   s.plan_tier || ' plan with ' || s.provider || ' (' || s.status
                       || ') in tenant ' || CAST(s.tenant_id AS TEXT)
            FROM subscriptions s
            WHERE s.user_id = $1
              AND s.status NOT IN ('canceled', 'incomplete_expired')
            ORDER BY kind, detail
            ";

/// Decode one [`DELETION_BLOCKERS_SQL`] row.
///
/// # Errors
/// Returns a database error when a column cannot be decoded, or an internal
/// error for a `kind` the model does not know (the statement and the enum
/// drifted apart).
pub fn user_reference_from_row<R>(row: &R) -> AppResult<UserReference>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let kind: String = row
        .try_get("kind")
        .map_err(|e| AppError::database(format!("Failed to get kind: {e}")))?;
    let detail: String = row
        .try_get("detail")
        .map_err(|e| AppError::database(format!("Failed to get detail: {e}")))?;
    Ok(UserReference {
        kind: kind.parse::<UserReferenceKind>()?,
        detail,
    })
}

/// The error a failed user delete reports.
///
/// A foreign-key violation means a row still references the user — one
/// [`DELETION_BLOCKERS_SQL`] could not see, or one written between that read
/// and the delete. That is a conflict the operator resolves by reassigning the
/// row, so it maps to [`ErrorCode::ResourceLocked`] (409), never to the 500 a
/// database fault gets. Both drivers classify the violation, so this reads
/// the same on either engine.
pub fn delete_user_error(user_id: Uuid, error: &sqlx::Error) -> AppError {
    let is_reference = error
        .as_database_error()
        .is_some_and(DatabaseError::is_foreign_key_violation);
    if is_reference {
        AppError::new(
            ErrorCode::ResourceLocked,
            format!("User {user_id} is still referenced by a row that does not cascade: {error}"),
        )
    } else {
        AppError::database(format!("Failed to delete user: {error}"))
    }
}
