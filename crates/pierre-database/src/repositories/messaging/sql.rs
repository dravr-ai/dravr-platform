// ABOUTME: The SQL the shared MessagingRepository body runs, bar the link-state and reaction statements beside it
// ABOUTME: `$n` placeholders and portable syntax, so SQLite and PostgreSQL run each statement as is
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ── Statements, written once for both backends ──
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres, so one statement serves both drivers and cannot drift between
// them. Where a table's `tenant_id`/`user_id` is a `uuid` column on Postgres
// (`messaging_sessions`, `messaging_channel_links`, `messaging_link_states`,
// `backfill_push_log`, and `messaging_outbound_queue.user_id`) the tenant
// binds as [`TenantId`], whose own sqlx encoding is text on `SQLite` and a
// native uuid on Postgres, and a user id the caller holds as text goes
// through the backend's uuid codec; where the column is `TEXT` on both
// (`messaging_channel_configs`, `messaging_messages`,
// `messaging_delivery_receipts`, `messaging_outbound_queue.tenant_id`) the
// tenant binds as its hyphenated string. Timestamps bind and read as
// `DateTime<Utc>` on both: `TIMESTAMPTZ` on Postgres, and on `SQLite` the
// RFC 3339 text these tables already hold. `TRUE`/`FALSE`, `ON CONFLICT …
// DO NOTHING` and `NULLS FIRST` are accepted by both engines.

/// The full channel-config projection, secrets included: what the webhook
/// authenticator and the per-tenant read need.
macro_rules! channel_config_columns {
    () => {
        "id, tenant_id, channel_type, api_key, api_secret, webhook_secret, \
         verify_token, account_id, phone_number, bot_token, is_active, created_at, updated_at"
    };
}

/// Insert or refresh the one config a tenant holds per channel type.
pub(crate) const UPSERT_CHANNEL_CONFIG_SQL: &str = r"
            INSERT INTO messaging_channel_configs
                (id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                 verify_token, account_id, phone_number, bot_token, is_active,
                 created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)
            ON CONFLICT(tenant_id, channel_type) DO UPDATE SET
                api_key = EXCLUDED.api_key,
                api_secret = EXCLUDED.api_secret,
                webhook_secret = EXCLUDED.webhook_secret,
                verify_token = EXCLUDED.verify_token,
                account_id = EXCLUDED.account_id,
                phone_number = EXCLUDED.phone_number,
                bot_token = EXCLUDED.bot_token,
                is_active = EXCLUDED.is_active,
                updated_at = EXCLUDED.updated_at
            ";

/// One tenant's config for one channel type.
pub(crate) const GET_CHANNEL_CONFIG_SQL: &str = concat!(
    "SELECT ",
    channel_config_columns!(),
    " FROM messaging_channel_configs WHERE tenant_id = $1 AND channel_type = $2"
);

/// Every config a tenant holds, without its secrets.
pub(crate) const LIST_CHANNEL_CONFIGS_SQL: &str = r"
            SELECT id, tenant_id, channel_type, is_active, created_at, updated_at
            FROM messaging_channel_configs
            WHERE tenant_id = $1
            ORDER BY channel_type
            ";

/// Every active config for a channel type, across tenants: the inbound
/// webhook carries no Pierre auth, so each tenant's signing secret is tried.
pub(crate) const CONFIGS_BY_CHANNEL_TYPE_SQL: &str = concat!(
    "SELECT ",
    channel_config_columns!(),
    " FROM messaging_channel_configs WHERE channel_type = $1 AND is_active = TRUE \
     ORDER BY created_at, id"
);

/// Whether another tenant's active config already carries one of the given
/// external identities. Each optional identity is one parameter, used twice.
pub(crate) const CHANNEL_IDENTITY_CLAIMED_SQL: &str = r"
            SELECT EXISTS(
                SELECT 1 FROM messaging_channel_configs
                WHERE channel_type = $1
                  AND is_active = TRUE
                  AND tenant_id <> $2
                  AND (
                      ($3 IS NOT NULL AND phone_number = $3)
                   OR ($4 IS NOT NULL AND account_id = $4)
                   OR ($5 IS NOT NULL AND bot_token = $5)
                  )
            )
            ";

/// Drop one tenant's config for one channel type.
pub(crate) const DELETE_CHANNEL_CONFIG_SQL: &str =
    "DELETE FROM messaging_channel_configs WHERE tenant_id = $1 AND channel_type = $2";

/// The config that serves a tenant on one channel: the tenant's own row when
/// it has one, otherwise the deployment's platform-scope row. Ranked in one
/// statement so there is a single rule, not a second lookup after a miss;
/// among platform rows (an `ADMIN_EMAIL` change can leave two) the most
/// recently seeded wins.
pub(crate) const RESOLVE_CHANNEL_CONFIG_SQL: &str = concat!(
    "SELECT ",
    channel_config_columns!(),
    " FROM messaging_channel_configs \
     WHERE channel_type = $2 AND (tenant_id = $1 OR platform_scope = TRUE) \
     ORDER BY CASE WHEN tenant_id = $1 THEN 0 ELSE 1 END, updated_at DESC, id \
     LIMIT 1"
);

/// Mark one tenant's config for one channel type as platform scope.
pub(crate) const MARK_CHANNEL_CONFIG_PLATFORM_SCOPE_SQL: &str = r"
            UPDATE messaging_channel_configs
               SET platform_scope = TRUE
             WHERE tenant_id = $1 AND channel_type = $2
            ";

/// The session projection every session read returns.
macro_rules! session_columns {
    () => {
        "id, user_id, tenant_id, channel_type, channel_user_id, \
         channel_conversation_id, pierre_conversation_id, last_message_at, created_at"
    };
}

/// Bind a channel identity to a Pierre conversation; `$8` stamps both
/// `last_message_at` and `created_at`.
pub(crate) const CREATE_SESSION_SQL: &str = r"
            INSERT INTO messaging_sessions
                (id, user_id, tenant_id, channel_type, channel_user_id,
                 channel_conversation_id, pierre_conversation_id, last_message_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
            ";

/// The session for a channel identity within one chat. `NULL` and the empty
/// string are the same chat, matching the unique-index expression in
/// migration `20260505000001_messaging_sessions_per_chat`.
pub(crate) const SESSION_BY_CHANNEL_IDENTITY_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM messaging_sessions \
     WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3 \
       AND COALESCE(channel_conversation_id, '') = COALESCE($4, '')"
);

/// The session that owns a Pierre conversation, within one tenant.
pub(crate) const SESSION_BY_CONVERSATION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM messaging_sessions WHERE tenant_id = $1 AND pierre_conversation_id = $2 LIMIT 1"
);

/// Stamp a session's last message instant.
pub(crate) const TOUCH_SESSION_SQL: &str =
    "UPDATE messaging_sessions SET last_message_at = $1 WHERE id = $2";

/// Repoint a session at a fresh Pierre conversation.
pub(crate) const SET_SESSION_CONVERSATION_SQL: &str =
    "UPDATE messaging_sessions SET pierre_conversation_id = $1 WHERE id = $2";

/// Record a message once per `(tenant_id, channel_message_id)`: the unique
/// index `idx_messaging_messages_idempotency` exists on both backends, and a
/// second delivery of the same channel message inserts nothing.
pub(crate) const INSERT_MESSAGE_SQL: &str = r"
            INSERT INTO messaging_messages
                (id, tenant_id, session_id, direction, channel_type, channel_message_id,
                 sender_id, content_type, content_body, correlation_id, raw_payload,
                 chat_message_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (tenant_id, channel_message_id) DO NOTHING
            ";

/// A session's messages, oldest first, one page at a time.
pub(crate) const SESSION_MESSAGES_SQL: &str = r"
            SELECT id, tenant_id, session_id, direction, channel_type, channel_message_id,
                   sender_id, content_type, content_body, correlation_id, raw_payload, created_at
            FROM messaging_messages
            WHERE session_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            LIMIT $3 OFFSET $4
            ";

/// Queue an outbound message as `pending` with no attempts; `$7` stamps
/// both `created_at` and `updated_at`.
pub(crate) const ENQUEUE_OUTBOUND_SQL: &str = r"
            INSERT INTO messaging_outbound_queue
                (id, message_id, tenant_id, user_id, channel_type, payload, status,
                 attempt_count, created_at, updated_at, expires_at, reauth_tenant_id,
                 reauth_provider)
            VALUES ($1, $2, $3, $4, $5, $6, 'pending', 0, $7, $7, $8, $9, $10)
            ";

/// The outbound-queue projection both pending reads return.
macro_rules! outbound_columns {
    () => {
        "id, message_id, tenant_id, user_id, channel_type, payload, status, \
         attempt_count, next_retry_at, created_at, updated_at, expires_at, \
         reauth_tenant_id, reauth_provider"
    };
}

/// Claim every tenant's due entries for the background retry worker.
///
/// The due-and-unleased condition is stated on the outer `UPDATE` as well as in
/// the subquery: on `PostgreSQL` a claim that waited for a concurrent one
/// re-checks the outer condition on each row it then reaches, so a row the
/// other claim just leased is skipped, not taken twice. `SQLite` serializes
/// writers.
pub(crate) const CLAIM_DUE_OUTBOUND_SQL: &str = concat!(
    "UPDATE messaging_outbound_queue \
     SET lease_token = $3, leased_until_ms = $4 \
     WHERE leased_until_ms <= $5 \
       AND (status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= $1)) \
       AND id IN ( \
         SELECT id FROM messaging_outbound_queue \
         WHERE leased_until_ms <= $5 \
           AND (status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= $1)) \
         ORDER BY next_retry_at ASC NULLS FIRST, created_at ASC \
         LIMIT $2) \
     RETURNING ",
    outbound_columns!(),
    ", lease_token"
);

/// Record the outcome of a send attempt by the claim that holds the entry,
/// and release its lease.
pub(crate) const UPDATE_OUTBOUND_STATUS_SQL: &str = r"
            UPDATE messaging_outbound_queue
            SET status = $1, attempt_count = $2, next_retry_at = $3, updated_at = $4,
                lease_token = NULL, leased_until_ms = 0
            WHERE id = $5 AND lease_token = $6
            ";

/// Bind a channel identity to a Pierre user for good.
pub(crate) const CREATE_CHANNEL_LINK_SQL: &str = r"
            INSERT INTO messaging_channel_links
                (id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ";

/// Re-key, step one: the user's link moves to the new channel identity, unless
/// a link in the tenant already holds that identity.
pub(crate) const REKEY_CHANNEL_LINK_SQL: &str = r"
            UPDATE messaging_channel_links
            SET channel_user_id = $1
            WHERE tenant_id = $2 AND channel_type = $3 AND channel_user_id = $4 AND user_id = $5
              AND NOT EXISTS (
                  SELECT 1 FROM messaging_channel_links held
                  WHERE held.tenant_id = $2 AND held.channel_type = $3
                    AND held.channel_user_id = $1
              )
            ";

/// Re-key, step two: the user's sessions keyed on the old identity follow the
/// link, under their own tenant (DMs) or the bot tenant.
pub(crate) const REKEY_CHANNEL_SESSIONS_SQL: &str = r"
            UPDATE messaging_sessions
            SET channel_user_id = $1
            WHERE tenant_id IN ($2, $3) AND user_id = $4 AND channel_type = $5
              AND channel_user_id = $6
            ";

/// Re-key, step three: the user's recorded turns follow the link, so a turn
/// recorded under the old identity still authenticates when it is resumed.
/// This table keeps its ids as text, so every id binds as a string.
pub(crate) const REKEY_RESUMABLE_TURNS_SQL: &str = r"
            UPDATE messaging_resumable_turns
            SET sender_id = $1
            WHERE tenant_id IN ($2, $3) AND channel_tenant_id = $3 AND user_id = $4
              AND channel_type = $5 AND sender_id = $6
            ";

/// The link behind a channel identity, within one tenant.
pub(crate) const GET_CHANNEL_LINK_SQL: &str = r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// The tenant that owns the earliest link for a channel identity, across
/// tenants: the backfill push runs under the athlete's tenant but sends
/// through the bot tenant's channel config.
pub(crate) const CHANNEL_LINK_TENANT_SQL: &str = r"
            SELECT tenant_id
            FROM messaging_channel_links
            WHERE channel_type = $1 AND channel_user_id = $2
            ORDER BY linked_at
            LIMIT 1
            ";

/// Every link a user holds, with the link's own locale override.
pub(crate) const LIST_USER_CHANNEL_LINKS_SQL: &str = r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, locale, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY linked_at
            ";

/// Every link a user holds under any tenant, with the link's own locale
/// override. Scoped by `user_id` alone: the row names the person, and a user
/// id belongs to exactly one account, so this can only ever return that
/// person's own links (see `list_channel_links_for_user`).
pub(crate) const LIST_CHANNEL_LINKS_FOR_USER_SQL: &str = r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, locale, linked_at
            FROM messaging_channel_links
            WHERE user_id = $1
            ORDER BY linked_at, id
            ";

/// Unlink one channel from a user.
pub(crate) const DELETE_CHANNEL_LINK_SQL: &str = r"
            DELETE FROM messaging_channel_links
            WHERE tenant_id = $1 AND user_id = $2 AND channel_type = $3
            ";

/// The per-link locale override, `NULL` when the link inherits the user's.
pub(crate) const CHANNEL_LINK_LOCALE_SQL: &str = r"
            SELECT locale
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// The locale override of a user's links on one channel type. `/language`
/// writes every such link at once, so they agree; the most recent link
/// answers for an athlete who linked a second identity afterwards.
pub(crate) const USER_CHANNEL_LINK_LOCALE_SQL: &str = r"
            SELECT locale
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND user_id = $2 AND channel_type = $3
              AND locale IS NOT NULL
            ORDER BY linked_at DESC
            LIMIT 1
            ";

/// Set or clear the per-link locale override.
pub(crate) const SET_CHANNEL_LINK_LOCALE_SQL: &str = r"
            UPDATE messaging_channel_links
               SET locale = $1
             WHERE tenant_id = $2 AND user_id = $3 AND channel_type = $4
            ";

/// Whether the one-time agent proposal has been stamped on a link.
pub(crate) const AGENT_PROPOSAL_SENT_SQL: &str = r"
            SELECT agent_proposal_sent_at IS NOT NULL
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// The agent ids the last proposal offered, as the JSON array it stored.
pub(crate) const PROPOSED_AGENT_IDS_SQL: &str = r"
            SELECT proposed_agent_ids FROM messaging_channel_links
             WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// Spend the proposal's offer once the athlete has picked from it.
pub(crate) const CLEAR_PROPOSED_AGENT_IDS_SQL: &str = r"
            UPDATE messaging_channel_links
               SET proposed_agent_ids = NULL
             WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// Stamp the proposal as sent and record what it offered, in order.
pub(crate) const MARK_AGENT_PROPOSAL_SENT_SQL: &str = r"
            UPDATE messaging_channel_links
               SET agent_proposal_sent_at = $1,
                   proposed_agent_ids = $2
             WHERE tenant_id = $3 AND channel_type = $4 AND channel_user_id = $5
            ";

/// Logout, step one: the link goes. Sessions and messages stay for support
/// and audit; without the link, `resolve_linked_session` never resumes them.
pub(crate) const LOGOUT_DELETE_LINK_SQL: &str = r"
            DELETE FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// Logout, step two: every open link state for the sender is spent.
pub(crate) const LOGOUT_INVALIDATE_STATES_SQL: &str = r"
            UPDATE messaging_link_states
            SET used = TRUE
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3 AND used = FALSE
            ";

/// The live in-chat OTP flow for a channel identity, newest first.
pub(crate) const ACTIVE_OTP_LINK_STATE_SQL: &str = r"
            SELECT id, tenant_id, user_id, channel_type, code, method,
                   channel_user_id, sender_name, otp_step, email, otp_hash,
                   otp_attempts, expires_at, created_at
            FROM messaging_link_states
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
              AND otp_step IS NOT NULL AND used = FALSE AND expires_at > $4
            ORDER BY created_at DESC
            LIMIT 1
            ";

/// Advance the flow to `awaiting_otp` with the address and the code hash.
pub(crate) const SET_OTP_ON_LINK_STATE_SQL: &str = r"
            UPDATE messaging_link_states
            SET email = $1, otp_hash = $2, otp_step = 'awaiting_otp', otp_attempts = 0
            WHERE id = $3
            ";

/// Park the flow on `awaiting_signup`: the address is known, no code sent.
pub(crate) const SET_SIGNUP_PENDING_SQL: &str = r"
            UPDATE messaging_link_states
            SET email = $1, otp_hash = NULL, otp_step = 'awaiting_signup', otp_attempts = 0
            WHERE id = $2
            ";

/// Count one more OTP attempt and hand back the new count in the same
/// statement, so two concurrent guesses cannot read the same number.
pub(crate) const INCREMENT_OTP_ATTEMPTS_SQL: &str = r"
            UPDATE messaging_link_states
            SET otp_attempts = otp_attempts + 1
            WHERE id = $1
            RETURNING otp_attempts
            ";

/// Spend every live OTP flow a sender has, before a new one starts.
pub(crate) const INVALIDATE_OTP_LINK_STATES_SQL: &str = r"
            UPDATE messaging_link_states
            SET used = TRUE
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
              AND otp_step IS NOT NULL AND used = FALSE
            ";

/// Claim a backfill-completion push window once across every replica: the
/// row is inserted iff the primary key is free, and one affected row means
/// this caller won.
pub(crate) const CLAIM_BACKFILL_PUSH_SQL: &str = r"
            INSERT INTO backfill_push_log
                (tenant_id, user_id, provider, after_ts)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (tenant_id, user_id, provider, after_ts) DO NOTHING
            ";
