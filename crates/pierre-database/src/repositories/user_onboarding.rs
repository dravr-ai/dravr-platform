// ABOUTME: Repository trait for durable per-user onboarding step state (profile-type, connect, agent, messaging)
// ABOUTME: Server-driven completion that survives device changes, replacing the web flow's localStorage-only flags
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use async_trait::async_trait;
use dravr_tronc::dates::{CivilDateTime, Timestamp};
use pierre_core::errors::{AppError, AppResult};

use super::tenants::{first_tenant_id_sql, session_tenants_sql};

/// One persisted onboarding step state for a user.
///
/// A record exists only for a step the user has reached; a step with no record is
/// still pending. `chosen_channel` is set only on the messaging-channel step (the
/// messaging app the user picked); it is `None` for every other step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingStepRecord {
    /// Step identifier (`profile_type`, `connect_provider`, `agent_proposal`,
    /// `messaging_channel`, `messaging_configure`).
    pub step_id: String,
    /// `complete`, `skipped` or `not_applicable`.
    pub status: String,
    /// The messaging channel chosen at the `messaging_channel` step, if any.
    pub chosen_channel: Option<String>,
    /// When the status was last written, from the engine clock; `None` when
    /// the stored value does not read as an instant. Home's Getting started
    /// reads it to tell a health check answered after the wizard from one
    /// answered in it, and when a row it ticks became done (carnet#1530).
    pub updated_at: Option<Timestamp>,
}

/// An athlete stuck at connect, read by the connect-stall sweep.
///
/// They answered the onboarding `profile_type` step and went no further: no
/// `connect_provider` step (connected, skipped or not applicable) and no real
/// provider connection in the athlete's tenant for that answer: the one it
/// was recorded in, or, when it was recorded with none, the athlete's
/// default tenant (the first of their memberships, where a session opens
/// when nothing names a tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StalledAtConnect {
    /// The athlete.
    pub user_id: String,
    /// The tenant the stall is in: the `profile_type` answer's, else the
    /// athlete's default tenant.
    pub tenant_id: String,
    /// When the `profile_type` answer was last written; `None` when the
    /// stored value does not read as an instant.
    pub profile_type_at: Option<Timestamp>,
}

/// Read a step's `updated_at` as the engines render it as text: `SQLite`
/// stores `YYYY-MM-DD HH:MM:SS` in UTC, Postgres renders `TIMESTAMPTZ` as
/// `YYYY-MM-DD HH:MM:SS[.ffffff]±HH[:MM]`. Anything else reads as `None`
/// rather than failing the whole list: the time only orders and dates rows.
#[must_use]
pub(crate) fn parse_step_time(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    if let Ok(at) = Timestamp::parse_from_rfc3339(text) {
        return Some(at);
    }
    if let Ok((at, _)) = Timestamp::parse_from_str_with_offset(text, "%Y-%m-%d %H:%M:%S%.f%#z") {
        return Some(at);
    }
    CivilDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .and_then(|at| at.and_utc().ok())
}

/// Durable onboarding progress, keyed by `(user_id, step_id)`.
///
/// Scoped by `user_id` — the auth-token-derived isolation boundary, mirroring the
/// provider-connection onboarding gate, which also reads by `user_id` alone.
/// `tenant_id` is a best-effort audit column (the onboarding JWT may carry no
/// active tenant), never a filter.
#[async_trait]
pub trait UserOnboardingRepository: Send + Sync {
    /// Upsert a step's `status` (and, for the messaging-channel step, the chosen
    /// channel) for a user. Re-marking an existing step overwrites its status.
    async fn set_onboarding_step(
        &self,
        user_id: &str,
        step_id: &str,
        status: &str,
        chosen_channel: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;

    /// Every recorded step state for a user (rows exist only for reached steps).
    async fn get_onboarding_steps(&self, user_id: &str) -> AppResult<Vec<OnboardingStepRecord>>;

    /// The athletes stalled at connect, most recent `profile_type` answer
    /// first, at most `limit` of them (see [`StalledAtConnect`]).
    ///
    /// The connect-stall sweep's cross-tenant scan: each answer is placed
    /// in one tenant (its own, else the athlete's default tenant) and
    /// checked against the provider connections of that tenant alone. An
    /// answer with no tenant from an account that belongs to none (so could
    /// hold no connection) is left out. The order is the engine's own, so
    /// the caller picks the time window from `profile_type_at`.
    async fn list_stalled_at_connect(&self, limit: i64) -> AppResult<Vec<StalledAtConnect>>;

    /// Return a user to the state onboarding starts from, in one transaction.
    ///
    /// Runs [`ONBOARDING_RESET_STATEMENTS`], then [`MEMORY_RESET_STATEMENTS`]
    /// when `scope` is [`OnboardingResetScope::WithMemory`]. Nothing a group
    /// reads is touched; see [`ONBOARDING_RESET_STATEMENTS`] for what counts
    /// as the user's own. Provider connections are not this method's to
    /// clear: they are revoked upstream through the disconnect chokepoint.
    async fn reset_onboarding(
        &self,
        user_id: &str,
        scope: OnboardingResetScope,
    ) -> AppResult<OnboardingReset>;
}

/// How much an onboarding reset clears, chosen by the admin per call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingResetScope {
    /// The step records, what onboarding captured, and the profile document.
    OnboardingOnly,
    /// That, plus everything the agents remember about the user: every fact,
    /// every agent note, and the user's own in-app conversations.
    WithMemory,
}

/// What an onboarding reset changed, by label, for the admin's report.
/// Labels with no affected row are absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OnboardingReset {
    /// Rows deleted or updated, keyed by the statement's label.
    pub rows_changed: BTreeMap<String, u64>,
}

/// A conversation that is the user's alone.
///
/// Theirs, in-app (the web and
/// mobile clients create with the column's `web` default, a `/reset` stamps
/// the client it was typed on, messaging threads carry their channel), bound
/// to no group, about no other athlete, and shared with nobody. A messaging
/// room keeps a per-member row, and an in-app thread can carry other
/// participants; either is somebody else's history too, so neither matches.
/// Nor does a human coach's thread bound to one athlete (carnet#1070): it is
/// work on that athlete's plan, kept like a room, and never the coach's own
/// thread for the platform to adopt or post into. The designated personal
/// thread always matches, whichever channel opened it: one adopted from the
/// athlete's messaging DM carries that channel (carnet#840).
macro_rules! personal_conversation {
    () => {
        "CAST(user_id AS TEXT) = $1 AND group_id IS NULL AND subject_user_id IS NULL \
         AND (channel_type IN ('web', 'mobile') OR personal = TRUE) \
         AND NOT EXISTS (SELECT 1 FROM conversation_participants p \
         WHERE p.conversation_id = chat_conversations.id AND CAST(p.user_id AS TEXT) <> $1)"
    };
}
pub(crate) use personal_conversation;

/// The literal of [`STEP_AGENT_CONSENT_PROMPT`], for the statements that
/// name it.
macro_rules! agent_consent_prompt_step {
    () => {
        "agent_consent_prompt"
    };
}

/// The `user_onboarding` step recording that the athlete answered the
/// one-time question about what Dravr's own agent may read (carnet#1544):
/// a consent decision kept beside the wizard's steps, never one of them.
pub const STEP_AGENT_CONSENT_PROMPT: &str = agent_consent_prompt_step!();

/// The onboarding half of a reset, as `(label, statement)` pairs.
///
/// Each binds the
/// user id as text to `$1` (every `user_id` here is `TEXT`, `VARCHAR` or
/// `uuid` depending on table and engine, and each renders the id the same).
///
/// - `user_onboarding`: every step record, so each wizard step is pending,
///   except [`STEP_AGENT_CONSENT_PROMPT`]: the athlete answered a one-time
///   consent question there, and a reset does not ask it again.
/// - `user_facts`: what onboarding wrote — the about-you and pillar answers
///   (`source = 'onboarding'`), anything placed under a pillar, the North Star,
///   and the PAR-Q medical flags — so pillar coverage reads zero again.
/// - `user_profiles`: the profile document the calibration and pillar stages
///   build, which `upsert_profile` replaces whole anyway.
/// - `chat_conversations.onboarding_state`: a guided walk left open in one of
///   the user's own threads, so it cannot resume half-way.
///
/// Group rows, memberships, invites, the ambient transcript, room threads,
/// `manages_roster` and the user's tier are never named here. Neither is
/// `users.coaching_persona`, which a notification gate reads and which the
/// profile-type step writes again when answered.
pub const ONBOARDING_RESET_STATEMENTS: &[(&str, &str)] = &[
    (
        "user_onboarding",
        concat!(
            "DELETE FROM user_onboarding WHERE CAST(user_id AS TEXT) = $1 AND step_id <> '",
            agent_consent_prompt_step!(),
            "'"
        ),
    ),
    (
        "user_facts (onboarding)",
        "DELETE FROM user_facts WHERE CAST(user_id AS TEXT) = $1 \
         AND (source = 'onboarding' OR pillar IS NOT NULL OR kind IN ('north_star', 'medical'))",
    ),
    (
        "user_profiles",
        "DELETE FROM user_profiles WHERE CAST(user_id AS TEXT) = $1",
    ),
    (
        "chat_conversations.onboarding_state",
        concat!(
            "UPDATE chat_conversations SET onboarding_state = NULL WHERE onboarding_state IS NOT NULL AND ",
            personal_conversation!()
        ),
    ),
];

/// The memory half of a reset.
///
/// Run after [`ONBOARDING_RESET_STATEMENTS`] when
/// the admin asks for it: every remaining fact and agent note about the
/// user, and their own in-app conversations (messages, verdicts and feedback
/// cascade with them). Room threads, threads shared with anyone else and a
/// coach's threads about the athletes they coach stay.
pub const MEMORY_RESET_STATEMENTS: &[(&str, &str)] = &[
    (
        "user_facts",
        "DELETE FROM user_facts WHERE CAST(user_id AS TEXT) = $1",
    ),
    (
        "agent_notes",
        "DELETE FROM agent_notes WHERE CAST(user_id AS TEXT) = $1",
    ),
    (
        "chat_conversations",
        concat!(
            "DELETE FROM chat_conversations WHERE ",
            personal_conversation!()
        ),
    ),
];

/// Upsert one step's state, stamping `updated_at` from the engine clock.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind is a plain `&str`/`Option<&str>`, so one statement
/// serves both backends and cannot drift between them. `CURRENT_TIMESTAMP` is
/// the portable spelling of the clock both engines already used under
/// different names.
pub(crate) const UPSERT_ONBOARDING_STEP_SQL: &str = r"
            INSERT INTO user_onboarding (user_id, step_id, status, chosen_channel, tenant_id, updated_at)
            VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP)
            ON CONFLICT (user_id, step_id) DO UPDATE SET
                status = excluded.status,
                chosen_channel = excluded.chosen_channel,
                tenant_id = excluded.tenant_id,
                updated_at = CURRENT_TIMESTAMP
            ";

/// Read every recorded step for a user. Scoped by `user_id` alone, which is the
/// trait's documented isolation boundary. `updated_at` is `TEXT` on `SQLite`
/// and `TIMESTAMPTZ` on Postgres, so it is read cast to text on both and
/// parsed by [`parse_step_time`]: one statement still serves both engines.
pub(crate) const SELECT_ONBOARDING_STEPS_SQL: &str = r"
            SELECT step_id, status, chosen_channel, CAST(updated_at AS TEXT) AS updated_at
            FROM user_onboarding
            WHERE user_id = $1
            ";

/// The connect-stall scan behind
/// [`UserOnboardingRepository::list_stalled_at_connect`].
///
/// Each answer is placed in one tenant: the one it was recorded in, else the
/// athlete's default tenant ([`first_tenant_id_sql`], the tenant a session
/// opens in when nothing names one), and the connection check runs in that
/// tenant alone. An answer that resolves to no tenant is left out. Every
/// column it compares is `TEXT` on both engines (the membership's `uuid`
/// columns are cast), and `updated_at` orders correctly on both
/// (`TIMESTAMPTZ`, and `SQLite`'s fixed-width UTC text), so one statement
/// serves both.
pub(crate) const SELECT_STALLED_AT_CONNECT_SQL: &str = concat!(
    "SELECT s.user_id, s.tenant_id, CAST(s.updated_at AS TEXT) AS updated_at \
     FROM (\
         SELECT p.user_id, p.updated_at, \
                COALESCE(p.tenant_id, (",
    first_tenant_id_sql!("p.user_id"),
    ")) AS tenant_id \
         FROM user_onboarding p \
         WHERE p.step_id = 'profile_type' \
           AND p.status = 'complete' \
           AND NOT EXISTS (\
               SELECT 1 FROM user_onboarding c \
               WHERE c.user_id = p.user_id AND c.step_id = 'connect_provider'\
           )\
     ) s \
     WHERE s.tenant_id IS NOT NULL \
       AND NOT EXISTS (\
           SELECT 1 FROM provider_connections pc \
           WHERE pc.user_id = s.user_id \
             AND pc.tenant_id = s.tenant_id \
             AND pc.connection_type <> 'synthetic'\
       ) \
     ORDER BY s.updated_at DESC \
     LIMIT $1"
);

/// Decode a row via `try_get` (never `Row::get`, which is `try_get().unwrap()`
/// and panics on a type/NULL surprise) so a corrupt row is a recoverable error.
///
/// Every column is TEXT on both engines, so these are plain `String` decodes
/// with no INT4-as-i64 or native-uuid trap, and one generic body serves both
/// drivers.
///
/// # Errors
/// Returns a database error when a column cannot be decoded.
pub(crate) fn step_record_from_row<R>(row: &R) -> AppResult<OnboardingStepRecord>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OnboardingStepRecord {
        step_id: row
            .try_get::<String, _>("step_id")
            .map_err(|e| AppError::database(format!("user_onboarding step_id: {e}")))?,
        status: row
            .try_get::<String, _>("status")
            .map_err(|e| AppError::database(format!("user_onboarding status: {e}")))?,
        chosen_channel: row
            .try_get::<Option<String>, _>("chosen_channel")
            .map_err(|e| AppError::database(format!("user_onboarding chosen_channel: {e}")))?,
        updated_at: row
            .try_get::<Option<String>, _>("updated_at")
            .map_err(|e| AppError::database(format!("user_onboarding updated_at: {e}")))?
            .as_deref()
            .and_then(parse_step_time),
    })
}

/// Decode one [`SELECT_STALLED_AT_CONNECT_SQL`] row; every column is `TEXT`.
///
/// # Errors
/// Returns a database error when a column cannot be decoded.
pub(crate) fn stalled_from_row<R>(row: &R) -> AppResult<StalledAtConnect>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(StalledAtConnect {
        user_id: row
            .try_get::<String, _>("user_id")
            .map_err(|e| AppError::database(format!("user_onboarding user_id: {e}")))?,
        tenant_id: row
            .try_get::<String, _>("tenant_id")
            .map_err(|e| AppError::database(format!("user_onboarding tenant_id: {e}")))?,
        profile_type_at: row
            .try_get::<Option<String>, _>("updated_at")
            .map_err(|e| AppError::database(format!("user_onboarding updated_at: {e}")))?
            .as_deref()
            .and_then(parse_step_time),
    })
}

/// Emit the whole [`UserOnboardingRepository`] implementation for one backend.
/// The body is written once here; each backend's shell invokes it with its own
/// type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_user_onboarding_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl UserOnboardingRepository for $ty {
            async fn set_onboarding_step(
                &self,
                user_id: &str,
                step_id: &str,
                status: &str,
                chosen_channel: Option<&str>,
                tenant_id: Option<&str>,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_ONBOARDING_STEP_SQL)
                    .bind(user_id)
                    .bind(step_id)
                    .bind(status)
                    .bind(chosen_channel)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert user_onboarding: {e}"))
                    })?;

                Ok(())
            }

            async fn get_onboarding_steps(
                &self,
                user_id: &str,
            ) -> AppResult<Vec<OnboardingStepRecord>> {
                let rows = sqlx::query(SELECT_ONBOARDING_STEPS_SQL)
                    .bind(user_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read user_onboarding: {e}"))
                    })?;

                rows.iter().map(step_record_from_row).collect()
            }

            async fn list_stalled_at_connect(
                &self,
                limit: i64,
            ) -> AppResult<Vec<StalledAtConnect>> {
                let rows = sqlx::query(SELECT_STALLED_AT_CONNECT_SQL)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to scan stalled connects: {e}"))
                    })?;

                rows.iter().map(stalled_from_row).collect()
            }

            async fn reset_onboarding(
                &self,
                user_id: &str,
                scope: OnboardingResetScope,
            ) -> AppResult<OnboardingReset> {
                let memory: &[(&str, &str)] = match scope {
                    OnboardingResetScope::OnboardingOnly => &[],
                    OnboardingResetScope::WithMemory => MEMORY_RESET_STATEMENTS,
                };
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin the onboarding reset: {e}"))
                })?;
                let mut reset = OnboardingReset::default();
                for (label, statement) in ONBOARDING_RESET_STATEMENTS.iter().chain(memory) {
                    let changed = sqlx::query(statement)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Onboarding reset failed on {label}: {e}"))
                        })?
                        .rows_affected();
                    if changed > 0 {
                        reset.rows_changed.insert((*label).to_owned(), changed);
                    }
                }
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to commit the onboarding reset: {e}"))
                })?;
                Ok(reset)
            }
        }
    };
}
pub(crate) use impl_user_onboarding_repository;

#[cfg(test)]
mod tests {
    use super::parse_step_time;
    use dravr_tronc::dates::Timestamp;

    #[test]
    fn a_step_time_reads_as_either_engine_renders_it() {
        let at = Timestamp::parse_from_rfc3339("2026-11-13T10:04:05Z").expect("instant");
        assert_eq!(
            parse_step_time("2026-11-13 10:04:05"),
            Some(at),
            "SQLite text, UTC"
        );
        assert_eq!(
            parse_step_time("2026-11-13 10:04:05+00"),
            Some(at),
            "Postgres, UTC session"
        );
        assert_eq!(
            parse_step_time("2026-11-13 05:04:05-05"),
            Some(at),
            "Postgres, another zone"
        );
        assert_eq!(
            parse_step_time("2026-11-13 15:34:05+05:30"),
            Some(at),
            "an offset with minutes"
        );
        assert_eq!(
            parse_step_time("2026-11-13 10:04:05.250000+00").map(Timestamp::epoch_millis),
            Some(at.epoch_millis() + 250),
            "fractional seconds"
        );
        assert_eq!(
            parse_step_time("2026-11-13T10:04:05Z"),
            Some(at),
            "RFC 3339"
        );
        assert_eq!(parse_step_time("yesterday"), None, "not an instant");
    }
}
