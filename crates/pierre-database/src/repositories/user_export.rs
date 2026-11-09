// ABOUTME: The tables an athlete's own data export reads, the columns it never reads, and the tables it leaves out with a reason each
// ABOUTME: One list drives both backends' page statements, so what is exported and what is withheld cannot drift apart
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What `GET /api/user/export` reads (carnet#1346, ADR-011).
//!
//! Law 25 s.27 and GDPR Article 20 give a person the personal information
//! they supplied, in a structured and commonly used format. The export is a
//! single JSON document of table name to rows, built from
//! [`USER_EXPORT_TABLES`]; it is **not** the OKF context bundle ADR-011
//! describes (`render_okf_bundle` renders what the assistant is shown, a
//! digest of the athlete's data), and it replaces nothing in the prompt path.
//! ADR-011's "the same export doubles as the Article 20 artifact" is satisfied
//! by this document instead: the stored rows themselves, not a rendering.
//!
//! # Included
//!
//! Every table that holds the athlete as the data subject:
//!
//! - the account and its settings (`users`, profile, preferences, units,
//!   onboarding, physiology, fitness configuration, consents, notification
//!   preferences, tenant memberships, federated identities);
//! - what providers delivered (connections, data sources, activities and their
//!   streams and routes, sleep, recovery, health snapshots, daily activity,
//!   personal bests, gear, uploaded-file records);
//! - what the athlete built or said (training plans and weeks, workouts,
//!   recipes, commitments, facts, notes, follow-ups, conversations and their
//!   messages, feedback, group membership and room transcript, messaging
//!   links, sessions and messages, agents they authored, the tasks their
//!   connected agents ran);
//! - what the service recorded about the account in their name (notifications,
//!   LLM usage, usage counters, subscription, consent and purge records, and
//!   the names and creation dates of their API keys and MCP tokens).
//!
//! # Excluded, and why
//!
//! - **Credentials**: password hash, OAuth access and refresh tokens, API-key
//!   and MCP-token hashes, session and refresh tokens, authorization codes,
//!   one-time codes, verification and reset tokens, push tokens, bring-your-own
//!   LLM and OAuth-app secrets, signing keys. Whole tables where the table is
//!   only a credential ([`USER_EXPORT_EXCLUDED_TABLES`]), named columns where
//!   the row is the athlete's ([`UserExportTable::excluded`]).
//! - **Raw bytes**: the original uploaded and pushed activity files. Their
//!   parsed content is exported (`cached_activities`, `activity_streams`); the
//!   bytes would make the response unbounded in size.
//! - **Operational state with no meaning outside the service**: sync cursors,
//!   backfill jobs and leases, queues, delivery receipts, rate-limit and tier
//!   overrides an operator set, worker bookkeeping.
//! - **Other people's records**: what a human coach wrote about the athlete
//!   (their thread, flags and dismissals), shared groups and their invites, an
//!   operator's own audit trail. Law 25 s.27 covers information collected
//!   from the person; these are the coach's and the operator's records.
//! - **Security and request logs** (`audit_events`, `jwt_usage`,
//!   `request_logs`): retained for abuse investigation; whether an access
//!   request must include them is a counsel question, not decided here.
//!
//! Every table a query reads is filtered by the athlete (`user_id`, or the
//! column naming them) and, where it has a `tenant_id`, by the tenants the
//! athlete belongs to. `tests/it/user_export_test.rs` holds this list
//! against the live schema: a table with a user-keyed column that is in
//! neither list, a column that looks like a credential and is not withheld, or
//! a binary column that is not withheld, fails the build.

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use serde_json::Value;
use uuid::Uuid;

/// One table the export reads.
#[derive(Debug, Clone, Copy)]
pub struct UserExportTable {
    /// The table, which is also the key its rows take in the export.
    pub name: &'static str,
    /// The column that scopes the table by tenant, when it has one. Rows in a
    /// tenant the athlete does not belong to are never read; a row with no
    /// tenant at all is the athlete's own and is.
    pub tenant_column: Option<&'static str>,
    /// Columns the export never carries: credentials and raw bytes.
    pub excluded: &'static [&'static str],
    /// Rows read per page. Tables whose rows carry a document (streams, route
    /// tracks) read few at a time so one page stays small.
    pub page_rows: i64,
    /// The table's primary key, which orders its pages. Unlike a row's position
    /// it does not move when the row is updated, so a row changed during an
    /// export is neither dropped nor repeated.
    pub key_columns: &'static [&'static str],
    /// The type of each column of [`Self::key_columns`], which the `PostgreSQL`
    /// statement casts a page's start to: `uuid`, `date` or `text`.
    pub key_kinds: &'static [&'static str],
    /// One page on `SQLite`: `$1` the athlete's id as text, `$2` the limit,
    /// `$3` the key the page starts after.
    pub sqlite_page_sql: &'static str,
    /// One page on `PostgreSQL`: `$1` the athlete's id as text, `$2` the limit,
    /// `$3` the key the page starts after, `$4` the columns to drop, each row
    /// as one `jsonb`. The page starts at its key through the primary key's
    /// index, and only the rows of the page are turned into documents, so a
    /// page costs the same however many rows the athlete has.
    pub postgres_page_sql: &'static str,
}

/// The alias of the page key a page statement returns beside each row's columns.
pub(crate) const USER_EXPORT_KEY_ALIAS_SQL: &str = "export_key";

/// One page of a table's rows.
#[derive(Debug, Default)]
pub struct UserExportPage {
    /// The rows, each a JSON object of its columns.
    pub rows: Vec<Value>,
    /// The key of the last row, which the next page starts after.
    pub last_key: Option<String>,
}

/// Read an athlete's rows one page at a time.
///
/// A page is read and serialized, then dropped before the next is read, so the
/// export holds one page of rows in memory however much data the athlete has.
#[async_trait]
pub trait UserExportRepository: Send + Sync {
    /// The rows of `table` that belong to `user_id`, `table.page_rows` at a
    /// time, starting after the key `after` (empty for the first page), each a
    /// JSON object of its columns without [`UserExportTable::excluded`]. Empty
    /// past the last row.
    async fn export_page(
        &self,
        table: &UserExportTable,
        user_id: Uuid,
        after: &str,
    ) -> AppResult<UserExportPage>;
}

/// `$1`, the athlete's id, written so it compares to a column of `$kind`
/// directly and the column's index stays usable. `SQLite` keeps every id as
/// text; `PostgreSQL` holds `uuid` and text id columns side by side.
macro_rules! athlete_id {
    (sqlite, $kind:tt) => {
        "$1"
    };
    (postgres, uuid) => {
        "CAST($1 AS uuid)"
    };
    (postgres, text) => {
        "$1"
    };
}

/// The tenants an athlete belongs to, as text: their memberships and the
/// tenant their account names.
macro_rules! member_tenants {
    ($backend:tt) => {
        concat!(
            "SELECT CAST(tu.tenant_id AS TEXT) FROM tenant_users tu WHERE tu.user_id = ",
            athlete_id!($backend, uuid),
            " UNION SELECT CAST(u.tenant_id AS TEXT) FROM users u WHERE u.id = ",
            athlete_id!($backend, uuid)
        )
    };
}

/// A column held to the athlete's tenants; a null tenant is the athlete's own.
macro_rules! within_member_tenants {
    ($backend:tt, $column:literal) => {
        concat!(
            "(",
            $column,
            " IS NULL OR CAST(",
            $column,
            " AS TEXT) IN (",
            member_tenants!($backend),
            "))"
        )
    };
}

/// The athlete's rows of a table: by a column naming them, of the type named,
/// or through a parent table the athlete owns inside their tenants.
macro_rules! athlete_rows {
    ($backend:tt, [uuid $column:literal]) => {
        concat!($column, " = ", athlete_id!($backend, uuid))
    };
    ($backend:tt, [text $column:literal]) => {
        concat!($column, " = ", athlete_id!($backend, text))
    };
    ($backend:tt, [conversations]) => {
        concat!(
            "conversation_id IN (SELECT c.id FROM chat_conversations c WHERE c.user_id = ",
            athlete_id!($backend, uuid),
            " AND ",
            within_member_tenants!($backend, "c.tenant_id"),
            ")"
        )
    };
    // A task carries no user column: `session_token` holds the athlete's own
    // key, or the id of a client they registered.
    ($backend:tt, [a2a_tasks]) => {
        concat!(
            "session_token = 'user:' || ",
            athlete_id!($backend, text),
            " OR session_token IN (SELECT c.client_id FROM a2a_clients c WHERE c.user_id = ",
            athlete_id!($backend, uuid),
            ")"
        )
    };
    ($backend:tt, [sessions]) => {
        concat!(
            "session_id IN (SELECT s.id FROM messaging_sessions s WHERE s.user_id = ",
            athlete_id!($backend, uuid),
            " AND ",
            within_member_tenants!($backend, "s.tenant_id"),
            ")"
        )
    };
    ($backend:tt, [sources]) => {
        concat!(
            "data_source_id IN (SELECT d.id FROM data_sources d WHERE d.user_id = ",
            athlete_id!($backend, text),
            " AND ",
            within_member_tenants!($backend, "d.tenant_id"),
            ")"
        )
    };
    ($backend:tt, [recipes]) => {
        concat!(
            "recipe_id IN (SELECT r.id FROM recipes r WHERE r.user_id = ",
            athlete_id!($backend, uuid),
            " AND ",
            within_member_tenants!($backend, "r.tenant_id"),
            ")"
        )
    };
}

/// The tenant filter for a table: none, or its column held to the athlete's
/// tenants.
macro_rules! tenant_filter {
    ($backend:tt, none) => {
        ""
    };
    ($backend:tt, $column:literal) => {
        concat!(" AND ", within_member_tenants!($backend, $column))
    };
}

/// The tenant column a filter names, if any.
macro_rules! tenant_column {
    (none) => {
        None
    };
    ($column:literal) => {
        Some($column)
    };
}

/// A primary-key column as text, in the `SQLite` page key.
macro_rules! key_column {
    ($column:literal) => {
        concat!("COALESCE(CAST(export_row.", $column, " AS TEXT), '')")
    };
}

/// A row's `SQLite` page key: its primary-key columns as one text value, joined
/// by a unit separator so two different keys cannot read the same.
macro_rules! sqlite_row_key {
    ($only:literal) => {
        key_column!($only)
    };
    ($first:literal, $($rest:literal),+) => {
        concat!(key_column!($first), " || char(31) || ", sqlite_row_key!($($rest),+))
    };
}

/// The primary-key columns of a `PostgreSQL` page, in key order.
macro_rules! postgres_key_columns {
    ($only:literal) => {
        concat!("export_row.", $only)
    };
    ($first:literal, $($rest:literal),+) => {
        concat!("export_row.", $first, ", ", postgres_key_columns!($($rest),+))
    };
}

/// The part of `$3`, a JSON array of the key columns, at position `$at` as the
/// column's own type, so the comparison reaches the column's index. `$3` is
/// empty on the first page, where the comparison is never decided by this.
macro_rules! postgres_key_part {
    (text, $at:literal) => {
        concat!("NULLIF($3, '')::jsonb->>", $at)
    };
    ($kind:ident, $at:literal) => {
        concat!(
            "CAST(NULLIF($3, '')::jsonb->>",
            $at,
            " AS ",
            stringify!($kind),
            ")"
        )
    };
}

/// What a `PostgreSQL` page's key columns compare to, in key order.
macro_rules! postgres_key_after {
    ($a:ident) => {
        postgres_key_part!($a, 0)
    };
    ($a:ident, $b:ident) => {
        concat!(postgres_key_part!($a, 0), ", ", postgres_key_part!($b, 1))
    };
    ($a:ident, $b:ident, $c:ident) => {
        concat!(
            postgres_key_part!($a, 0),
            ", ",
            postgres_key_part!($b, 1),
            ", ",
            postgres_key_part!($c, 2)
        )
    };
    ($a:ident, $b:ident, $c:ident, $d:ident) => {
        concat!(
            postgres_key_part!($a, 0),
            ", ",
            postgres_key_part!($b, 1),
            ", ",
            postgres_key_part!($c, 2),
            ", ",
            postgres_key_part!($d, 3)
        )
    };
}

/// Declares [`USER_EXPORT_TABLES`], building each table's page statements at
/// compile time from its predicate, so no statement is assembled at run time.
macro_rules! user_export_tables_sql {
    ($(
        $name:literal {
            rows: $rows:tt,
            tenant: $tenant:tt,
            except: [$($excluded:literal),*],
            page: $page:literal,
            key: [$($kind:ident $key:literal),+] $(,)?
        }
    ),+ $(,)?) => {
        /// Every table the export reads, in the order it writes them.
        pub const USER_EXPORT_TABLES: &[UserExportTable] = &[$(
            UserExportTable {
                name: $name,
                tenant_column: tenant_column!($tenant),
                excluded: &[$($excluded),*],
                page_rows: $page,
                key_columns: &[$($key),+],
                key_kinds: &[$(stringify!($kind)),+],
                sqlite_page_sql: concat!(
                    "SELECT export_row.*, ", sqlite_row_key!($($key),+), " AS ", "export_key",
                    " FROM ", $name, " AS export_row WHERE (", athlete_rows!(sqlite, $rows), ")",
                    tenant_filter!(sqlite, $tenant),
                    " AND ", sqlite_row_key!($($key),+), " > $3",
                    " ORDER BY export_key LIMIT $2"
                ),
                // The inner statement picks the page and carries each row's
                // columns as stored; only the outer one reads their documents.
                // In one statement the database builds a document for every
                // row it sorts, every page.
                postgres_page_sql: concat!(
                    "SELECT to_jsonb(page) - $4::text[] - 'export_key' AS row, page.export_key",
                    " FROM (SELECT export_row.*, jsonb_build_array(",
                    postgres_key_columns!($($key),+), ")::text AS export_key",
                    " FROM ", $name, " AS export_row WHERE (", athlete_rows!(postgres, $rows), ")",
                    tenant_filter!(postgres, $tenant),
                    " AND ($3 = '' OR (", postgres_key_columns!($($key),+), ") > (",
                    postgres_key_after!($($kind),+), "))",
                    " ORDER BY ", postgres_key_columns!($($key),+), " LIMIT $2) AS page"
                ),
            }
        ),+];
    };
}

user_export_tables_sql!(
    // The account. The row is the athlete's by its id alone: the tenant it names
    // may be null, and a null must not drop the account from its own export.
    "users" { rows: [uuid "id"], tenant: none, except: ["password_hash"], page: 5, key: [uuid "id"] },
    "tenant_users" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [uuid "id"] },
    "user_profiles" { rows: [uuid "user_id"], tenant: none, except: [], page: 5, key: [uuid "user_id"] },
    "user_configurations" { rows: [uuid "user_id"], tenant: none, except: [], page: 5, key: [uuid "user_id"] },
    "user_home_preferences" { rows: [uuid "user_id"], tenant: none, except: [], page: 5, key: [uuid "user_id"] },
    "user_unit_preferences" { rows: [uuid "user_id"], tenant: none, except: [], page: 5, key: [uuid "user_id"] },
    "user_check_in_preferences" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 5, key: [uuid "tenant_id", uuid "user_id"] },
    "user_onboarding" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "user_id", text "step_id"] },
    "calendar_preferences" { rows: [uuid "user_id"], tenant: "tenant_id", except: ["set_by"], page: 5, key: [text "tenant_id", uuid "user_id"] },
    "user_physiological_profiles" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 5, key: [uuid "tenant_id", uuid "user_id"] },
    "fitness_configurations" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 20, key: [text "id"] },
    "goals" { rows: [uuid "user_id"], tenant: none, except: [], page: 50, key: [text "id"] },
    "user_agent_preferences" { rows: [uuid "user_id"], tenant: none, except: [], page: 100, key: [text "id"] },
    "first_run_progress" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [uuid "tenant_id", uuid "user_id", text "flow_id", text "local_date"] },
    "use_case_exposures" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [uuid "tenant_id", uuid "user_id", text "use_case_id"] },
    "user_federated_identities" { rows: [uuid "user_id"], tenant: none, except: [], page: 20, key: [text "provider", text "subject"] },
    "provider_terms_consents" { rows: [uuid "user_id"], tenant: none, except: [], page: 50, key: [uuid "user_id", text "provider"] },
    "signup_code_redemptions" { rows: [uuid "user_id"], tenant: none, except: [], page: 20, key: [uuid "user_id"] },
    "notification_preferences" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "scheduled_notifications" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "oauth_client_grants" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "impersonation_sessions" { rows: [uuid "target_user_id"], tenant: none, except: ["impersonator_id"], page: 50, key: [text "id"] },
    "permission_delegations" { rows: [uuid "grantor_id"], tenant: none, except: [], page: 50, key: [text "id"] },
    // What providers delivered.
    "provider_connections" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "provider_data_purges" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "data_sources" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "synthetic_activities" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "activity_gear" { rows: [uuid "user_id"], tenant: none, except: [], page: 200, key: [uuid "user_id", text "provider", text "activity_id"] },
    "athlete_gear" { rows: [uuid "user_id"], tenant: none, except: [], page: 100, key: [uuid "id"] },
    "personal_best_efforts" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [uuid "user_id", uuid "tenant_id", text "distance"] },
    "uploaded_activity_files" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "tenant_id", text "user_id", text "file_sha256"] },
    "pushed_activity_files" { rows: [text "user_id"], tenant: "tenant_id", except: ["file_bytes"], page: 200, key: [text "tenant_id", text "user_id", text "provider", text "activity_id"] },
    "sleep_sessions" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "recovery_metrics" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "id"] },
    "health_snapshots" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "id"] },
    "daily_activity" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "id"] },
    "activity_conversations" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "tenant_id", text "user_id", text "provider", text "activity_id"] },
    // Training.
    "training_history" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 200, key: [uuid "tenant_id", uuid "user_id", date "date"] },
    "training_plans" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 10, key: [text "id"] },
    "training_plan_weeks" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 25, key: [text "id"] },
    "prescribed_workouts" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 50, key: [uuid "id"] },
    "workout_templates" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 25, key: [uuid "id"] },
    "recipes" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "recipe_ingredients" { rows: [recipes], tenant: none, except: [], page: 200, key: [text "id"] },
    // What the athlete said and what the agents kept about them.
    "agents" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 10, key: [text "id"] },
    "agent_authors" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 5, key: [text "id"] },
    "agent_assignments" { rows: [uuid "user_id"], tenant: none, except: [], page: 100, key: [text "id"] },
    "agent_sessions" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "agent_notes" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "agent_followups" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "athlete_commitments" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "pending_advice" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "coaching_playbooks" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "claim_verdicts" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "user_facts" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "id"] },
    "chat_conversations" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    "chat_message_feedback" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "id"] },
    "conversation_participants" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 200, key: [text "conversation_id", uuid "user_id"] },
    "group_transcript_entries" { rows: [uuid "author_user_id"], tenant: "tenant_id", except: [], page: 100, key: [uuid "id"] },
    "coaching_group_members" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [uuid "id"] },
    "coach_access_requests" { rows: [uuid "user_id"], tenant: none, except: [], page: 100, key: [text "id"] },
    "delegated_connections" { rows: [uuid "member_user_id"], tenant: "member_tenant_id", except: [], page: 100, key: [uuid "id"] },
    "feedback_messages" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    // Messaging.
    "messaging_channel_links" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "messaging_sessions" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    // What the service recorded in their name.
    "notifications" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 200, key: [uuid "id"] },
    "oauth_notifications" { rows: [uuid "user_id"], tenant: none, except: [], page: 200, key: [text "id"] },
    "device_tokens" { rows: [text "user_id"], tenant: "tenant_id", except: ["expo_push_token"], page: 50, key: [text "id"] },
    "usage_counters" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 500, key: [text "tenant_id", text "user_id", text "counter_key", text "period"] },
    "subscriptions" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 20, key: [uuid "id"] },
    "user_llm_credentials_audit" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 100, key: [text "id"] },
    // The names and dates of the athlete's keys; never the keys.
    "api_keys" { rows: [uuid "user_id"], tenant: none, except: ["key_hash"], page: 50, key: [text "id"] },
    "user_mcp_tokens" { rows: [uuid "user_id"], tenant: none, except: ["token_hash"], page: 50, key: [text "id"] },
    "a2a_clients" { rows: [uuid "user_id"], tenant: none, except: [], page: 50, key: [text "client_id"] },
    // The tables that grow with the athlete's history, last, in rough order of
    // how much each row weighs.
    // The file stops at its size or time limit, so everything above is written
    // before these, however long that history is: per-activity terrain,
    // conversation summaries, messaging and chat turns, one row per model call,
    // what an agent the athlete connected asked and was answered (each row
    // holds the task's whole input and output, and unlike `mcp_tasks` a row
    // does not expire; a tool task is in no conversation), provider sample
    // series, the activities themselves, then their route tracks and
    // per-second streams, the largest rows the export reads.
    "route_summaries" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 25, key: [uuid "tenant_id", uuid "user_id", text "activity_id"] },
    "compaction_blocks" { rows: [conversations], tenant: "tenant_id", except: [], page: 50, key: [text "id"] },
    "messaging_messages" { rows: [sessions], tenant: "tenant_id", except: ["raw_payload"], page: 100, key: [text "id"] },
    "llm_usage" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 500, key: [text "id"] },
    "a2a_tasks" { rows: [a2a_tasks], tenant: none, except: ["session_token"], page: 25, key: [text "task_id"] },
    "chat_messages" { rows: [conversations], tenant: none, except: [], page: 50, key: [text "id"] },
    "data_point_series" { rows: [sources], tenant: none, except: [], page: 500, key: [text "id"] },
    "cached_activities" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 25, key: [text "id"] },
    "activity_route_tracks" { rows: [text "user_id"], tenant: "tenant_id", except: [], page: 5, key: [text "tenant_id", text "user_id", text "provider", text "activity_id"] },
    "activity_streams" { rows: [uuid "user_id"], tenant: "tenant_id", except: [], page: 5, key: [text "tenant_id", uuid "user_id", text "provider", text "activity_id"] },
);

/// Tables holding a user-keyed column that the export does not read, each with
/// the reason. A table with such a column is in this list or in
/// [`USER_EXPORT_TABLES`]; the catalog test fails otherwise.
pub const USER_EXPORT_EXCLUDED_TABLES: &[(&str, &str)] = &[
    // Credentials.
    (
        "user_oauth_tokens",
        "credential: provider access and refresh tokens",
    ),
    (
        "user_oauth_app_credentials",
        "credential: the athlete's own OAuth app secret",
    ),
    (
        "user_llm_credentials",
        "credential: the athlete's own LLM API key",
    ),
    (
        "session_refresh_tokens",
        "credential: session refresh tokens",
    ),
    ("oauth2_refresh_tokens", "credential: OAuth refresh tokens"),
    (
        "oauth2_auth_codes",
        "credential: single-use authorization codes",
    ),
    ("oauth2_states", "credential: in-flight OAuth state"),
    (
        "oauth_client_states",
        "credential: in-flight provider OAuth state and PKCE verifier",
    ),
    (
        "email_verification_tokens",
        "credential: verification tokens",
    ),
    ("password_reset_tokens", "credential: reset tokens"),
    (
        "website_sign_in_tokens",
        "credential: single-use sign-in tokens",
    ),
    (
        "messaging_link_states",
        "credential: link codes and one-time passwords",
    ),
    ("a2a_sessions", "credential: session tokens"),
    // Operational state.
    ("activity_backfill_coverage", "operational: sync progress"),
    (
        "activity_backfill_jobs",
        "operational: queued jobs and leases",
    ),
    (
        "activity_fetch_failures",
        "operational: sync failure counters",
    ),
    (
        "activity_fetch_freshness",
        "operational: sync freshness stamps",
    ),
    ("backfill_push_log", "operational: sync log"),
    ("best_effort_scans", "operational: scan markers"),
    ("personal_best_seeds", "operational: scan cursors"),
    ("sync_state", "operational: sync cursors"),
    (
        "guardian_pending_actions",
        "operational: transient tool-call approvals",
    ),
    ("mcp_tasks", "operational: transient protocol tasks"),
    ("messaging_outbound_queue", "operational: delivery queue"),
    ("messaging_resumable_turns", "operational: turn queue"),
    (
        "push_receipts_pending",
        "operational: push delivery tickets",
    ),
    ("short_links", "operational: transient links"),
    ("persona_digest_returns", "operational: digest bookkeeping"),
    ("platform_post_claims", "operational: post claims"),
    (
        "strava_seat_reclaim_warnings",
        "operational: seat-reclaim bookkeeping",
    ),
    (
        "user_feature_overrides",
        "operator configuration, not collected from the athlete",
    ),
    (
        "user_rate_limit_overrides",
        "operator configuration, not collected from the athlete",
    ),
    (
        "user_tier_overrides",
        "operator configuration, not collected from the athlete",
    ),
    (
        "user_tool_overrides",
        "operator configuration, not collected from the athlete",
    ),
    // Logs.
    (
        "audit_events",
        "security log: counsel decides whether access requests include it",
    ),
    (
        "jwt_usage",
        "request log: counsel decides whether access requests include it",
    ),
    (
        "request_logs",
        "request log: counsel decides whether access requests include it",
    ),
    // Operator records.
    ("admin_config_audit", "operator audit trail"),
    ("admin_config_overrides", "operator configuration"),
    // Other people's records.
    (
        "coach_reply_reviews",
        "a human coach's flags and corrections of replies",
    ),
    (
        "coach_attention_dismissals",
        "a human coach's own dismissals",
    ),
    (
        "coaching_groups",
        "a shared room; its members are other people",
    ),
    (
        "agents_orphaned",
        "PostgreSQL only: agents quarantined for naming no tenant",
    ),
    ("admin_tokens", "credential: operator service tokens"),
    (
        "tenant_tool_overrides",
        "operator configuration of a tenant",
    ),
];

/// Column-name fragments that mark a likely credential. The catalog test fails
/// a column carrying one that the export neither withholds nor lists in
/// [`NOT_CREDENTIAL_COLUMNS`].
pub const CREDENTIAL_MARKERS: &[&str] = &[
    "password",
    "secret",
    "token",
    "hash",
    "key",
    "credential",
    "otp",
    "nonce",
    "salt",
    "pem",
    "verifier",
    "encrypted",
];

/// Columns that match [`CREDENTIAL_MARKERS`] and are not credentials: token
/// counts, a key's public prefix, an idempotency key.
pub const NOT_CREDENTIAL_COLUMNS: &[&str] = &[
    "prompt_tokens",
    "completion_tokens",
    "total_tokens",
    "cached_tokens",
    "cached_write_tokens",
    "cached_write_1h_tokens",
    "reasoning_tokens",
    "token_count",
    "token_prefix",
    "key_prefix",
    "idempotency_key",
    // Content digests and ids of records, not secrets.
    "gpx_hash",
    "payload_hash",
    "content_hash",
    "trigger_hash",
    "intervention_hash",
    "summary_tokens",
    "original_tokens",
    "counter_key",
    "credential_id",
];
