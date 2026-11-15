// ABOUTME: A whole-account reset in one transaction: every row the user owns goes, the account and its sign-in stay
// ABOUTME: Every user-keyed table is wiped or kept with a reason, and the wipe is read back before it commits

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Starting an account over without deleting it (carnet#1463).
//!
//! A reset keeps the `users` row — the id, the email, the role, the tier and
//! the way the account signs in — and removes everything the account owns, so
//! the next sign-in lands on an account with no history. Nothing about it is
//! a deletion: no tombstone is written (the same provider account and email
//! may connect again at once), no deletion receipt is chained, and the
//! Firebase identity is left at Google.
//!
//! The delete leans on `DELETE FROM users` cascading to some forty tables. A
//! reset never deletes that row, so no cascade from it ever fires, and the
//! delete's own list ([`super::user_references::UserPurge`]) — which names
//! only what the cascade misses — is no use here. This module carries its own
//! inventory instead, held against the live catalog by a schema test: every
//! table with a user-keyed column (`user_id`, `*_user_id`, `owner_id`, `*_by`)
//! is either wiped ([`SQLITE_ACCOUNT_RESET`], [`POSTGRES_ACCOUNT_RESET`]) or
//! kept ([`RESET_KEEP`]) with the reason it stays.
//!
//! The wipe runs in ONE transaction, in the order the statements are listed:
//! a row that references another wiped row with no `ON DELETE` goes first. It
//! then reads every wiped table again with the same predicates and refuses to
//! commit while a row survives. Provider tokens and connections are not the
//! wipe's: the service in front of it (`pierre_services::account_reset`) has
//! already disconnected every provider this build can revoke through the
//! disconnect chokepoint, which proves its own rows gone, and keeps the ones
//! it cannot revoke so their grants are not orphaned at the provider.
//!
//! The reset refuses, before anything is touched, while the account holds
//! something other people use ([`RESET_BLOCKERS_SQL`]). Those are a subset of
//! the delete's blockers: the ones that exist only because the `users` row
//! disappears (operator attribution, sole tenant ownership, a live billing
//! subscription) do not apply to a reset, which keeps that row and those rows.
//!
//! The history dravr-barrage imported for the account and the files it
//! uploaded live outside this transaction's reach. The service erases them
//! itself before the wipe runs, rather than queueing the account on the
//! delete's outbox ([`super::archive_erasures`]): the account stays live, so a
//! sweep reaching the row later could not tell what it held at the reset from
//! what it uploaded or imported since, and would erase the new bytes under
//! rows that name them.
//!
//! ISOLATION EXEMPTION: every statement here is scoped by the user id alone,
//! across every tenant, never by `tenant_id`. That is deliberate and mirrors
//! the account delete's `UserPurge`: an account is one person in every tenant
//! it belongs to, and a reset of it reaches every tenant's rows. Do not add a
//! tenant filter; it would leave the account's rows in its other tenants.

use std::collections::BTreeMap;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{TenantId, UserReference};
use sqlx::error::DatabaseError;
use uuid::Uuid;

/// The rows of a table the account owns by its own `user_id` column: the
/// predicate every wipe entry takes unless it names one.
macro_rules! reset_rows {
    () => {
        "CAST(user_id AS TEXT) = $1"
    };
    ($predicate:literal) => {
        $predicate
    };
}

/// The tables one engine's reset wipes, and the statements it runs over them,
/// in the order it runs them.
#[derive(Debug, Clone, Copy)]
pub struct AccountResetWipe {
    /// Every table wiped, in order; each is also the label its count is
    /// reported under.
    pub tables: &'static [&'static str],
    /// One `DELETE` per entry of [`Self::tables`], in its order, each binding
    /// the user id as text to `$1`.
    pub statements: &'static [&'static str],
    /// The wiped tables that still hold a row for the account, with the
    /// count, read inside the reset's transaction before it commits.
    pub surviving: &'static str,
}

/// One engine's [`AccountResetWipe`], built from `table [where predicate]`
/// entries.
macro_rules! reset_wipe {
    ($($table:literal $(where $predicate:literal)?),+) => {
        AccountResetWipe {
            tables: &[$($table),+],
            statements: &[$(
                concat!("DELETE FROM ", $table, " WHERE ", reset_rows!($($predicate)?))
            ),+],
            surviving: concat!(
                "SELECT tbl, n FROM (SELECT '' AS tbl, 0 AS n WHERE 1 = 0",
                $(
                    " UNION ALL SELECT '", $table, "', COUNT(*) FROM ", $table,
                    " WHERE ", reset_rows!($($predicate)?)
                ),+,
                ") survivors WHERE n > 0"
            ),
        }
    };
}

/// Declares what a reset wipes from one list, so the reset, its survivor read
/// and the list a schema test holds against the live catalog cannot name
/// different tables.
///
/// `both` holds the tables both schemas carry; `postgres_only` the ones only
/// the `PostgreSQL` migrations create, which the `SQLite` reset must not name.
/// Each predicate casts the column it compares to text and binds the id as its
/// hyphenated text, which every `user_id` type of either engine renders it as.
macro_rules! account_reset_tables {
    (
        both: [$($table:literal $(where $predicate:literal)?),+ $(,)?],
        postgres_only: [$($pg_table:literal $(where $pg_predicate:literal)?),+ $(,)?] $(,)?
    ) => {
        /// The tables only the `PostgreSQL` schema carries that the reset
        /// wipes, after [`SQLITE_ACCOUNT_RESET`]'s on that engine alone.
        ///
        /// `agents_orphaned` is the quarantine a `PostgreSQL`-only migration
        /// moved agents with an unknown tenant into: the author's copies, with
        /// no constraint that would take them along.
        pub const POSTGRES_ONLY_RESET_TABLES: &[&str] = &[$($pg_table),+];

        /// What the `SQLite` reset wipes: every table both engines' reset
        /// wipes, in the order it wipes them.
        ///
        /// Each is the account's own data, matched by the user id across every
        /// tenant. Where the account's rows are reached through a parent
        /// rather than a column of their own, the entry names the predicate
        /// that reaches them, and the children come before the parent:
        ///
        /// - Chat: the conversations the account owns (`user_id`) and shares
        ///   with nobody, with their messages, compaction blocks, verdicts,
        ///   feedback, participants, activity threads and agent
        ///   introductions, before the conversations themselves. A thread the
        ///   account owns and another user takes part in is that user's
        ///   history too, as an onboarding reset holds: it stays, with every
        ///   message in it (a message names no author), the other
        ///   participants' seats and the account's own owner seat. A private
        ///   agent such a thread runs on refuses the reset
        ///   ([`RESET_BLOCKERS_SQL`]). The account's own rows go wherever they
        ///   were written: every verdict said to it (one an MCP client's
        ///   `verify_claim` wrote names no conversation, or another one) and
        ///   every rating it left, in its threads or in someone else's. A human
        ///   coach's thread *about* the athlete (`subject_user_id`) and the
        ///   rows of other people's threads that are about the athlete
        ///   (`about_user_id`) are the coach's record and stay; so do a group
        ///   room's per-member rows of other members. The account's
        ///   participation in someone else's shared thread goes with it, like
        ///   its group memberships.
        /// - Groups: the account's memberships, the room transcript it wrote,
        ///   the reply reviews and attention dismissals on either side of it,
        ///   and the delegated connections it is the member or the coach of.
        ///   An active group the account owns or coaches, or one with an open
        ///   invite it created, refuses the reset ([`RESET_BLOCKERS_SQL`]). An
        ///   archived group the account owns that nobody else used — no other
        ///   user's conversation is bound to it and no other user was ever a
        ///   member — is the account's own leftover and goes, once the
        ///   account's conversations and memberships are gone and before the
        ///   agents: its invites, transcript, digests and reviews cascade with
        ///   it. An archived group someone else used stays as their room
        ///   record.
        /// - Agents: the private agents the account authored (a shared one
        ///   refuses the reset), after every conversation and group bound to
        ///   them, with their assignments, authorship and preferences.
        /// - Messaging: the outbound retries, delivery receipts and messages of
        ///   the account's sessions, then the sessions (their foreign keys have
        ///   no `ON DELETE`), the resumable turns, and the channel links: a
        ///   `WhatsApp` or Slack identity has to be linked again.
        /// - Health: `data_point_series` has no user column and is reached
        ///   through the account's `data_sources`; sleep, recovery, snapshots
        ///   and daily activity reference those sources with no `ON DELETE`,
        ///   so all of them go before the sources.
        /// - A2A: the `OAuth2` registrations of the account's A2A clients
        ///   (keyed by the client's id, naming no user, so nothing else would
        ///   stop their secrets obtaining tokens), the tasks naming the
        ///   account or its clients, its sessions, then the clients.
        /// - Plans: by the athlete's `user_id` only, never by
        ///   `drafted_by_user_id` — a coach's drafts of another athlete's plan
        ///   are that athlete's season.
        /// - `memory_extraction_jobs` names its user only inside the JSON
        ///   payload a turn queued (`ExtractionJobPayload`, serialized compact,
        ///   so `"user_id":"<id>"` is exact). A job a turn still running queues
        ///   after this commits is not reached; it writes facts the next reset
        ///   clears.
        pub const SQLITE_ACCOUNT_RESET: AccountResetWipe =
            reset_wipe!($($table $(where $predicate)?),+);

        /// What the `PostgreSQL` reset wipes: [`SQLITE_ACCOUNT_RESET`]'s
        /// tables, then [`POSTGRES_ONLY_RESET_TABLES`].
        pub const POSTGRES_ACCOUNT_RESET: AccountResetWipe = reset_wipe!(
            $($table $(where $predicate)?),+,
            $($pg_table $(where $pg_predicate)?),+
        );
    };
}

account_reset_tables!(
    both: [
    "coaching_group_members",
    "delegated_connections" where "CAST(member_user_id AS TEXT) = $1 OR CAST(coach_user_id AS TEXT) = $1",
    "provider_terms_consents",
    "strava_seat_reclaim_warnings",
    "oauth_notifications",
    "oauth2_clients" where "client_id IN (SELECT c.client_id FROM a2a_clients c WHERE CAST(c.user_id AS TEXT) = $1)",
    "a2a_tasks" where "session_token = 'user:' || $1 OR session_token IN (SELECT c.client_id FROM a2a_clients c WHERE CAST(c.user_id AS TEXT) = $1)",
    "a2a_sessions",
    "a2a_clients",
    "push_receipts_pending",
    "device_tokens",
    "persona_digest_returns",
    "notifications",
    "notification_preferences",
    "scheduled_notifications",
    "messaging_outbound_queue" where "CAST(user_id AS TEXT) = $1 OR message_id IN (SELECT m.id FROM messaging_messages m JOIN messaging_sessions s ON s.id = m.session_id WHERE CAST(s.user_id AS TEXT) = $1)",
    "messaging_delivery_receipts" where "message_id IN (SELECT m.id FROM messaging_messages m JOIN messaging_sessions s ON s.id = m.session_id WHERE CAST(s.user_id AS TEXT) = $1)",
    "messaging_messages" where "session_id IN (SELECT s.id FROM messaging_sessions s WHERE CAST(s.user_id AS TEXT) = $1)",
    "messaging_sessions",
    "messaging_resumable_turns",
    "messaging_channel_links",
    "messaging_link_states",
    "activity_conversations" where "CAST(user_id AS TEXT) = $1 OR conversation_id IN (SELECT c.id FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))",
    "agent_introductions" where "thread_id IN (SELECT CAST(c.id AS TEXT) FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))",
    "chat_message_feedback" where "CAST(user_id AS TEXT) = $1 OR conversation_id IN (SELECT c.id FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))",
    "claim_verdicts" where "CAST(user_id AS TEXT) = $1 OR conversation_id IN (SELECT c.id FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))",
    "compaction_blocks" where "conversation_id IN (SELECT c.id FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))",
    "chat_messages" where "conversation_id IN (SELECT c.id FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))",
    "conversation_participants" where "(CAST(user_id AS TEXT) = $1 AND conversation_id NOT IN (SELECT c.id FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))) OR conversation_id IN (SELECT c.id FROM chat_conversations c WHERE CAST(c.user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = c.id AND CAST(p.user_id AS TEXT) <> $1))",
    "chat_conversations" where "CAST(user_id AS TEXT) = $1 AND NOT EXISTS (SELECT 1 FROM conversation_participants p WHERE p.conversation_id = chat_conversations.id AND CAST(p.user_id AS TEXT) <> $1)",
    "coach_reply_reviews" where "CAST(athlete_user_id AS TEXT) = $1 OR CAST(coach_user_id AS TEXT) = $1",
    "coach_attention_dismissals" where "CAST(athlete_user_id AS TEXT) = $1 OR CAST(coach_user_id AS TEXT) = $1",
    "group_transcript_entries" where "CAST(author_user_id AS TEXT) = $1",
    "coaching_groups" where "CAST(owner_id AS TEXT) = $1 AND is_active = FALSE AND NOT EXISTS (SELECT 1 FROM chat_conversations c WHERE c.group_id = coaching_groups.id OR c.subject_group_id = coaching_groups.id) AND NOT EXISTS (SELECT 1 FROM coaching_group_members m WHERE m.group_id = coaching_groups.id AND CAST(m.user_id AS TEXT) <> $1)",
    "coach_access_requests",
    "short_links",
    "mcp_tasks",
    "guardian_pending_actions",
    "agent_followups",
    "agent_notes",
    "agent_sessions",
    "athlete_commitments",
    "coaching_playbooks",
    "pending_advice",
    "user_facts",
    "memory_extraction_jobs" where "CAST(payload AS TEXT) LIKE '%\"user_id\":\"' || $1 || '\"%'",
    "user_onboarding",
    "first_run_progress",
    "use_case_exposures",
    "platform_post_claims",
    "feedback_messages",
    "user_physiology_sport_anchors",
    "user_physiological_profiles",
    "user_profiles",
    "user_configurations",
    "user_unit_preferences",
    "user_home_preferences",
    "user_check_in_preferences",
    "calendar_preferences",
    "coach_data_reads",
    "user_agent_preferences",
    "agent_assignments",
    "agent_authors",
    "agents",
    "fitness_configurations",
    "goals",
    "training_plan_weeks" where "CAST(user_id AS TEXT) = $1 OR plan_id IN (SELECT p.id FROM training_plans p WHERE CAST(p.user_id AS TEXT) = $1)",
    "training_plans",
    "training_history",
    "prescribed_workouts",
    "workout_templates",
    "route_summaries",
    "recipes",
    "synthetic_activities",
    "activity_gear",
    "athlete_gear",
    "best_effort_scans",
    "personal_best_efforts",
    "personal_best_seeds",
    "cached_activities",
    "uploaded_activity_files",
    "activity_route_tracks",
    "activity_streams",
    "pushed_activity_files",
    "activity_backfill_coverage",
    "activity_backfill_jobs",
    "activity_fetch_freshness",
    "activity_fetch_failures",
    "backfill_push_log",
    "sleep_sessions",
    "recovery_metrics",
    "health_snapshots",
    "daily_activity",
    "data_point_series" where "data_source_id IN (SELECT d.id FROM data_sources d WHERE CAST(d.user_id AS TEXT) = $1)",
    "data_sources",
    "sync_state",
    ],
    postgres_only: ["agents_orphaned"],
);

/// Every user-keyed table a reset keeps, with the reason it stays.
///
/// The account itself, the way it signs in and what operators set on it stay,
/// so the reset account is the same account; the ledgers stay because they
/// are records of what happened, not the account's data.
pub const RESET_KEEP: &[(&str, &str)] = &[
    ("users", "the account itself: id, email, role, tier, status and sign-in; approved_by and manages_roster_granted_by are operator attribution (a TrainingPeaks coach disconnect may revoke an earned manages_roster through the chokepoint)"),
    ("tenant_users", "tenant membership and role: the account stays in its tenants"),
    ("user_tier_overrides", "an operator's tier override on the account"),
    ("user_rate_limit_overrides", "an operator's rate-limit override on the account"),
    ("user_feature_overrides", "an operator's feature flags on the account"),
    ("user_tool_overrides", "an operator's tool overrides on the account"),
    ("admin_config_overrides", "operator configuration scoped to the account, and attribution of overrides the account wrote as an operator"),
    ("admin_config_audit", "the operator configuration audit trail"),
    ("admin_tokens", "operator device tokens the account approved (attribution, no foreign key)"),
    ("device_authorization", "pierre-cli device-login approvals the account made as an operator"),
    ("user_federated_identities", "sign-in identity: the account keeps signing in the same way"),
    ("session_refresh_tokens", "live app sessions: a reset does not sign the account out"),
    ("user_mcp_tokens", "the account's MCP client tokens (sign-in credentials)"),
    ("website_sign_in_tokens", "short-lived sign-in tokens"),
    ("email_verification_tokens", "sign-in state"),
    ("password_reset_tokens", "sign-in state"),
    ("permission_delegations", "role and permission grants"),
    ("impersonation_sessions", "the operator record of impersonations"),
    ("signup_code_redemptions", "how the account was created"),
    ("signup_codes", "codes the account minted as an operator (attribution)"),
    ("pre_approved_emails", "the sign-up allow-list and who allowed each email"),
    ("user_invitations", "invitations the account sent as an operator"),
    ("user_llm_credentials", "bring-your-own LLM credentials: account configuration"),
    ("user_llm_credentials_audit", "the audit trail of those credentials"),
    ("user_oauth_app_credentials", "the account's own provider OAuth app configuration, not provider data"),
    ("user_oauth_tokens", "cleared only by the disconnect chokepoint, which ran first for every provider this build can revoke and proved its rows gone; what remains belongs to a provider this build cannot revoke, kept and reported so its grant is not orphaned at the provider"),
    ("provider_connections", "cleared only by the disconnect chokepoint, as user_oauth_tokens; a remaining row is a provider this build cannot revoke, kept and reported"),
    ("provider_data_purges", "the per-provider attestation a provider's terms ask for that its data was purged, which the reset's own disconnects write"),
    ("api_keys", "the account's own API keys (sign-in credentials)"),
    ("request_logs", "request telemetry of the account's own API keys, which stay"),
    ("jwt_usage", "authentication telemetry"),
    ("oauth2_auth_codes", "OAuth server grants of the account's connected MCP clients (sign-in)"),
    ("oauth2_refresh_tokens", "OAuth server grants of the account's connected MCP clients (sign-in)"),
    ("oauth2_states", "OAuth server state of the account's connected MCP clients (sign-in)"),
    ("oauth_client_states", "OAuth client authorization state"),
    ("oauth_client_grants", "OAuth client authorization grants"),
    ("audit_events", "the admin audit trail, the record of this reset included"),
    ("usage_counters", "quota metering: a reset is not a new quota"),
    ("llm_usage", "the LLM cost ledger"),
    ("subscriptions", "billing state: the provider keeps billing whatever the reset does"),
    ("archive_erasures", "the account delete's erasure outbox; a reset erases the account's imported history and uploads itself, before its wipe, and queues nothing here"),
    ("group_invites", "an invite the account created to an active group refuses the reset; one to an archived group the reset wipes cascades with it; one to an archived group someone else used is inert and stays with it"),
    ("tenant_oauth_credentials", "a tenant's provider OAuth app the account configured as an operator (attribution)"),
    ("tenant_feature_defaults", "tenant feature defaults the account set as an operator (attribution)"),
    ("tenant_tool_overrides", "tenant tool overrides the account enabled as an operator (attribution)"),
    ("agent_versions", "attribution on other authors' agent versions; the versions of the account's own agents go with them"),
    ("store_listings", "store review decisions the account made as an operator (attribution)"),
    ("import_tombstones", "the deletion ledger's tombstones and who claimed or cleared them"),
];

/// Every reference onto one account a reset refuses over, as `(kind,
/// detail)` rows, read before anything is disconnected.
///
/// A subset of the delete's blockers (`DELETION_BLOCKERS_SQL`): what a reset
/// would take from other people. An active coaching group the account owns or
/// coaches, an invite it created to an active group, and an agent it authored
/// that is shared beyond it, bound to a group the reset keeps, to another
/// user's conversation or to a thread the account shares with another user
/// (both kept), or to another user's team membership.
///
/// The reset reads these twice: before any provider is disconnected, so a
/// refusal touches nothing, and again inside the wipe's transaction before
/// its first `DELETE`, so one made while the providers were being
/// disconnected still refuses rather than wiping an agent someone now relies
/// on (no foreign key would catch it: nothing that binds an agent references
/// it).
///
/// `taken_groups` names the archived groups the reset wipes itself, with the
/// same rule as its `coaching_groups` entry: owned by the account, archived,
/// no conversation the reset keeps bound to it (another user's, or one the
/// account shares with another user) and no other user's membership in it. An agent bound only to such groups does not block. Any other group
/// binding does, since the reset keeps that group and it would name an agent
/// the reset wipes.
///
/// The delete's other blockers do not apply: operator and audit attribution,
/// LLM credentials made for others, approvals, and sole ownership of a tenant
/// refuse a delete only because the `users` row they reference disappears,
/// and a live subscription only because its row would; a reset keeps both.
/// `$2` reads the blockers as they will stand once the reset has archived
/// every active group the account owns (`--delete-owned-groups`): those
/// groups, and the coach and invite references onto them, no longer refuse,
/// and they count as archived for `taken_groups`. A group the account
/// coaches or an invite it made to a group someone else owns still refuses.
/// The read inside the wipe's transaction binds `FALSE`: by then the archive
/// has run, and a group made since must still refuse.
///
/// The `kind` literals are `UserReferenceKind::as_str`; `$1` is the account id
/// through the backend's uuid codec.
pub const RESET_BLOCKERS_SQL: &str = r"
            WITH taken_groups AS (
                SELECT g.id
                FROM coaching_groups g
                WHERE g.owner_id = $1
                  AND (g.is_active = FALSE OR $2)
                  AND NOT EXISTS (
                      SELECT 1 FROM chat_conversations c
                      WHERE (c.group_id = g.id OR c.subject_group_id = g.id)
                        AND (c.user_id <> $1
                             OR EXISTS (
                                 SELECT 1 FROM conversation_participants p
                                 WHERE p.conversation_id = c.id AND p.user_id <> $1)))
                  AND NOT EXISTS (
                      SELECT 1 FROM coaching_group_members m
                      WHERE m.group_id = g.id AND m.user_id <> $1))
            SELECT 'owns_coaching_group' AS kind, g.name AS detail
            FROM coaching_groups g
            WHERE g.owner_id = $1 AND g.is_active = TRUE AND NOT $2
            UNION ALL
            SELECT 'coaches_coaching_group', g.name
            FROM coaching_groups g
            WHERE g.coach_user_id = $1 AND g.is_active = TRUE
              AND NOT ($2 AND g.owner_id = $1)
            UNION ALL
            SELECT 'created_group_invite', g.name
            FROM group_invites i
            JOIN coaching_groups g ON g.id = i.group_id
            WHERE i.created_by = $1 AND g.is_active = TRUE
              AND NOT ($2 AND g.owner_id = $1)
            UNION ALL
            SELECT 'authored_agent', a.title
            FROM agents a
            WHERE a.user_id = $1
              AND (a.visibility <> 'private'
                   OR EXISTS (
                       SELECT 1 FROM chat_conversations c
                       WHERE c.agent_id = a.id
                         AND (c.user_id <> $1
                              OR EXISTS (
                                  SELECT 1 FROM conversation_participants p
                                  WHERE p.conversation_id = c.id AND p.user_id <> $1)))
                   OR EXISTS (
                       SELECT 1 FROM coaching_groups g
                       WHERE g.agent_id = a.id
                         AND g.id NOT IN (SELECT t.id FROM taken_groups t))
                   OR EXISTS (
                       SELECT 1 FROM agent_assignments s
                       WHERE s.agent_id = a.id AND s.user_id <> $1
                         AND s.is_member = TRUE))
            ORDER BY kind, detail
            ";

/// The active groups the account owns, in every tenant, which
/// `--delete-owned-groups` archives before the reset runs; `$1` is the
/// account id through the backend's uuid codec. Ids are read as text so one
/// decoder serves both engines.
pub(crate) const RESET_OWNED_GROUPS_SQL: &str = r"
            SELECT CAST(g.id AS TEXT) AS group_id, CAST(g.tenant_id AS TEXT) AS tenant_id, g.name
            FROM coaching_groups g
            WHERE g.owner_id = $1 AND g.is_active = TRUE
            ORDER BY g.name, g.id
            ";

/// An active group the account owns, as a reset archives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedGroup {
    /// The group's id.
    pub group_id: String,
    /// The tenant the group lives in, which its archive is scoped by.
    pub tenant_id: TenantId,
    /// The group's name, for the operator.
    pub name: String,
}

/// Whether the account exists, `$1` its id through the backend's uuid codec.
pub(crate) const RESET_ACCOUNT_EXISTS_SQL: &str = "SELECT 1 AS present FROM users WHERE id = $1";

/// What a committed reset changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountReset {
    /// Rows deleted per wiped table; a table with none is absent.
    pub rows_changed: BTreeMap<String, u64>,
}

/// How a reset's transaction ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountResetCommit {
    /// Every row the account owns was wiped and the transaction committed.
    Reset(AccountReset),
    /// The account holds something other people use ([`RESET_BLOCKERS_SQL`],
    /// read again inside the transaction): nothing was wiped.
    Blocked(Vec<UserReference>),
}

/// A whole-account reset, and the references that refuse it.
#[async_trait]
pub trait AccountResetRepository: Send + Sync {
    /// Every reference onto `user_id` that refuses a reset
    /// ([`RESET_BLOCKERS_SQL`]); empty when the reset may run.
    /// `owned_groups_archived` reads them as they will stand once every
    /// active group the account owns is archived.
    ///
    /// # Errors
    /// Returns a database error when the rows cannot be read or decoded.
    async fn reset_blockers(
        &self,
        user_id: Uuid,
        owned_groups_archived: bool,
    ) -> AppResult<Vec<UserReference>>;

    /// The active groups `user_id` owns, in every tenant
    /// ([`RESET_OWNED_GROUPS_SQL`]).
    ///
    /// # Errors
    /// Returns a database error when the rows cannot be read or decoded.
    async fn owned_active_groups(&self, user_id: Uuid) -> AppResult<Vec<OwnedGroup>>;

    /// Wipe every row the account owns, in one transaction, as the module
    /// documentation describes: the blockers read again, the engine's wipe
    /// list in order, then the survivor read. A blocker found then ends the
    /// transaction before its first `DELETE`
    /// ([`AccountResetCommit::Blocked`]).
    ///
    /// # Errors
    /// Returns a not-found error for an unknown account, the conflict a row
    /// still referencing a wiped row raises (`ErrorCode::ResourceLocked`), an
    /// internal error when a wiped table still holds a row for the account,
    /// and a database error for any other failure. Nothing is committed then.
    async fn reset_account(&self, user_id: Uuid) -> AppResult<AccountResetCommit>;
}

/// The error a failed reset statement reports.
///
/// A foreign-key violation means a row still references what the reset
/// clears — one [`RESET_BLOCKERS_SQL`] could not see, or one written between
/// that read and the reset. That is a conflict an operator resolves, so it
/// maps to [`ErrorCode::ResourceLocked`] (409), never to a 500.
pub(crate) fn reset_error(user_id: Uuid, error: &sqlx::Error) -> AppError {
    let is_reference = error
        .as_database_error()
        .is_some_and(DatabaseError::is_foreign_key_violation);
    if is_reference {
        AppError::new(
            ErrorCode::ResourceLocked,
            format!(
                "User {user_id} reset hit a row that references what it clears and does not cascade: {error}"
            ),
        )
    } else {
        AppError::database(format!("Failed to reset user {user_id}: {error}"))
    }
}

/// One [`RESET_OWNED_GROUPS_SQL`] row.
pub(crate) fn owned_group_from_row<R>(row: &R) -> AppResult<OwnedGroup>
where
    R: sqlx::Row,
    for<'c> &'c str: sqlx::ColumnIndex<R>,
    for<'r> String: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    let text = |column: &str| {
        row.try_get::<String, _>(column)
            .map_err(|e| AppError::database(format!("Failed to read the group's {column}: {e}")))
    };
    Ok(OwnedGroup {
        group_id: text("group_id")?,
        tenant_id: TenantId::parse_str(&text("tenant_id")?)
            .map_err(|e| AppError::database(format!("Failed to read the group's tenant: {e}")))?,
        name: text("name")?,
    })
}

/// Emit the whole [`AccountResetRepository`] implementation for one backend
/// type: `$ids` is its uuid codec and `$wipe` its [`AccountResetWipe`]. The
/// invoking shell must `use` every statement and helper named here.
macro_rules! impl_account_reset_repository {
    ($ty:ty, $ids:ident, $wipe:ident) => {
        #[async_trait::async_trait]
        impl AccountResetRepository for $ty {
            async fn reset_blockers(
                &self,
                user_id: Uuid,
                owned_groups_archived: bool,
            ) -> AppResult<Vec<UserReference>> {
                let rows = sqlx::query(RESET_BLOCKERS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(owned_groups_archived)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read user reset blockers: {e}"))
                    })?;
                rows.iter().map(user_reference_from_row).collect()
            }

            async fn owned_active_groups(&self, user_id: Uuid) -> AppResult<Vec<OwnedGroup>> {
                let rows = sqlx::query(RESET_OWNED_GROUPS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the account's groups: {e}"))
                    })?;
                rows.iter().map(owned_group_from_row).collect()
            }

            async fn reset_account(&self, user_id: Uuid) -> AppResult<AccountResetCommit> {
                let owner = user_id.to_string();
                let tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin the account reset: {e}"))
                })?;
                let mut guard = TransactionGuard::new(tx);

                sqlx::query(RESET_ACCOUNT_EXISTS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(guard.executor()?)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read the account: {e}")))?
                    .ok_or_else(|| AppError::not_found(format!("User {user_id} not found")))?;

                // Read again here: a share made since the service's first
                // read (while its providers were being disconnected) must
                // still refuse, and nothing that binds an agent to other
                // people is a foreign key that would refuse the delete.
                let blockers = sqlx::query(RESET_BLOCKERS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(false)
                    .fetch_all(guard.executor()?)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read user reset blockers: {e}"))
                    })?
                    .iter()
                    .map(user_reference_from_row)
                    .collect::<AppResult<Vec<UserReference>>>()?;
                if !blockers.is_empty() {
                    return Ok(AccountResetCommit::Blocked(blockers));
                }

                let mut rows_changed = BTreeMap::new();
                for (table, statement) in $wipe.tables.iter().zip($wipe.statements) {
                    let removed = sqlx::query(statement)
                        .bind(&owner)
                        .execute(guard.executor()?)
                        .await
                        .map_err(|e| reset_error(user_id, &e))?
                        .rows_affected();
                    if removed > 0 {
                        rows_changed.insert((*table).to_owned(), removed);
                    }
                }

                let survivors = sqlx::query($wipe.surviving)
                    .bind(&owner)
                    .fetch_all(guard.executor()?)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to verify the account reset: {e}"))
                    })?;
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
                        "User {user_id} reset left rows behind in {}; nothing was committed",
                        tables.join(", ")
                    )));
                }

                guard.commit().await?;
                Ok(AccountResetCommit::Reset(AccountReset { rows_changed }))
            }
        }
    };
}
pub(crate) use impl_account_reset_repository;
