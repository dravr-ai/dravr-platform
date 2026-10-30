// ABOUTME: A human coach's draft of an athlete's training plan — its types, its statements and its in-transaction helpers
// ABOUTME: The draft is an overlay on the live season that no athlete read selects; publish replays it through the live save path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coach's draft (carnet#1069)
//!
//! A group's human coach writes a coached athlete's plan into a row of
//! `training_plans` in status `draft`, under the athlete's tenant and user.
//! Every athlete read selects the `active` season, and the week listings
//! serve only the weeks of the season and its history, so nothing the coach
//! drafts reaches the athlete until the coach publishes it.
//!
//! The draft is an overlay, not a copy: its row holds an outline — the one
//! the coach re-laid, or the live one copied verbatim — and only the weeks
//! the coach saved hang off it. The first version of each drafted week names
//! the live week it was written over in `supersedes_id` (never flipping it),
//! which is how a publish tells the weeks the athlete changed since from the
//! ones they did not touch. Publishing replays the draft through the live
//! save path in one transaction and marks it `published`; discarding marks
//! it `discarded`.
//!
//! The statements and helpers here are shared by both backends: the helpers
//! are emitted per backend by [`impl_training_plan_draft_helpers`], beside
//! the repository the parent module's macro emits.

use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::transport::TransportPolicy;
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use sqlx::error::DatabaseError;

use super::{PlanAuthor, PlanOutlineInput, PlanWeekInput, SaveTrainingPlanParams};

/// A human coach's open draft of an athlete's plan (carnet#1069).
///
/// The draft is an overlay on the athlete's live season, not a copy of it:
/// [`Self::plan`] holds an outline — the one the coach re-laid, or the live
/// one copied verbatim — and only the weeks the coach saved hang off it
/// ([`list_draft_weeks`](super::TrainingPlanRepository::list_draft_weeks)).
/// The athlete never reads it: every athlete read selects the `active`
/// season.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanDraft {
    /// The draft's outline, in status `draft`. `created_at` is when the draft
    /// began, `updated_at` when the coach last saved into it.
    pub plan: TrainingPlan,
    /// The coach who owns the draft.
    pub drafted_by_user_id: String,
    /// The live season the draft began from; `None` when the athlete had
    /// none. A live season with another id was re-laid under the draft.
    pub base_plan_id: Option<String>,
    /// Whether the coach re-laid the outline, so that publishing re-lays the
    /// season; otherwise the outline is a copy of the live one and publishing
    /// puts the drafted weeks on the live season as it stands.
    pub outline_drafted: bool,
}

/// Whose draft a call reads or writes: the athlete's tenant and user, where
/// the draft lives beside their season, and the coach who owns it.
#[derive(Debug, Clone, Copy)]
pub struct DraftOwner<'a> {
    /// The athlete's tenant.
    pub tenant_id: &'a str,
    /// The athlete.
    pub user_id: &'a str,
    /// The coach.
    pub coach_user_id: &'a str,
}

/// A save into a coach's draft: an optional outline plus zero or more weeks,
/// committed in one transaction
/// ([`save_draft_bundle`](super::TrainingPlanRepository::save_draft_bundle)).
#[derive(Debug)]
pub struct SaveDraftParams<'a> {
    /// Whose draft.
    pub owner: DraftOwner<'a>,
    /// The agent the coach's turn answers as: recorded on the outline it lays
    /// and on every week it writes.
    pub author: PlanAuthor<'a>,
    /// Pillar `Goal` user-fact a re-laid outline serves, when linked.
    pub goal_fact_id: Option<&'a str>,
    /// An outline re-laying the draft's season, or `None` to draft weeks over
    /// the outline the draft already holds — the live one, copied, on a
    /// draft's first save.
    pub outline: Option<PlanOutlineInput<'a>>,
    /// Weeks to draft, each superseding the draft's own version of its
    /// `week_start`.
    pub weeks: &'a [PlanWeekInput<'a>],
    /// The stamp of what this save was derived from (carnet#769).
    pub transport_policy: TransportPolicy,
}

/// Result of [`save_draft_bundle`](super::TrainingPlanRepository::save_draft_bundle).
#[derive(Debug)]
pub struct SavedDraftBundle {
    /// The draft as it stands after the save.
    pub draft: PlanDraft,
    /// The weeks this save wrote into it, in input order.
    pub weeks: Vec<PlanWeek>,
    /// Whether this save opened the draft.
    pub opened: bool,
}

/// What publishing a draft would overwrite on the athlete's side.
///
/// A week the coach never drafted is never here: it keeps the athlete's
/// version whatever the athlete did to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftConflicts {
    /// The live season is no longer the one the draft began from: it was
    /// re-laid, laid for the first time, or is gone. The draft's phase
    /// indexes, and a copied outline, may no longer fit it.
    pub season_relaid: bool,
    /// The weeks (`week_start`, ascending) the coach drafted whose live
    /// version changed after the coach drafted over it — the athlete's agent
    /// saved it again, or saved it for the first time.
    pub weeks: Vec<String>,
}

impl DraftConflicts {
    /// Whether publishing would overwrite nothing of the athlete's.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.season_relaid && self.weeks.is_empty()
    }
}

/// A draft and what publishing it now would overwrite.
#[derive(Debug, Clone)]
pub struct DraftPreview {
    /// The open draft.
    pub draft: PlanDraft,
    /// What it collides with.
    pub conflicts: DraftConflicts,
}

/// A publish request ([`publish_draft`](super::TrainingPlanRepository::publish_draft)).
#[derive(Debug)]
pub struct PublishDraftParams<'a> {
    /// Whose draft.
    pub owner: DraftOwner<'a>,
    /// Publish over the conflicts the draft has.
    pub overwrite: bool,
    /// The goal fact a re-laid season links to, when the caller converged
    /// one; otherwise the draft's own.
    pub goal_fact_id: Option<&'a str>,
}

/// A draft that is now the athlete's live plan.
#[derive(Debug)]
pub struct PublishedDraft {
    /// The draft row, now in status `published`.
    pub draft_id: String,
    /// The athlete's live season after the publish: a new outline when the
    /// draft re-laid it, else the season the weeks were put on.
    pub plan: TrainingPlan,
    /// The live weeks the publish wrote, one per drafted week.
    pub weeks: Vec<PlanWeek>,
    /// The season this publish superseded, when it re-laid one.
    pub superseded_plan_id: Option<String>,
    /// Whether the publish laid an outline.
    pub outline_published: bool,
}

/// What [`publish_draft`](super::TrainingPlanRepository::publish_draft) came to.
#[derive(Debug)]
pub enum DraftPublish {
    /// The draft is live.
    Published(PublishedDraft),
    /// The draft collides with the athlete's own changes and the caller did
    /// not ask to overwrite them. Nothing was written.
    Conflicted(DraftPreview),
    /// The coach holds no open draft for the athlete.
    NoDraft,
}

/// The coach's open draft of the athlete's plan. The one-draft index
/// guarantees at most one row.
pub(crate) const DRAFT_PLAN_SQL: &str = concat!(
    "SELECT ",
    plan_columns!(),
    " FROM training_plans \
     WHERE tenant_id = $1 AND user_id = $2 AND drafted_by_user_id = $3 AND status = 'draft'"
);

/// The coach's open draft, read by writing it, which takes the row lock on
/// Postgres and the write lock on `SQLite`, so two saves into one draft, or a
/// save and a publish, run one after the other. A save stamps `updated_at`
/// with `$4`; a publish passes NULL and leaves it, so a refused publish
/// reports the draft as the coach last saved it.
pub(crate) const CLAIM_DRAFT_SQL: &str = concat!(
    "UPDATE training_plans SET updated_at = COALESCE($4, updated_at) \
     WHERE tenant_id = $1 AND user_id = $2 AND drafted_by_user_id = $3 AND status = 'draft' \
     RETURNING ",
    plan_columns!()
);

/// Open a draft whose outline the coach lays (`outline_drafted`), over the
/// live season `$17` when there is one.
pub(crate) const INSERT_DRAFT_SQL: &str = concat!(
    "INSERT INTO training_plans (",
    plan_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'draft', NULL, $10, $11, $11, $12, $13, $14, $15, \
     $16, $17, TRUE) RETURNING ",
    plan_columns!()
);

/// Open a draft over the athlete's live season by copying its outline
/// verbatim — the JSON is carried, never re-encoded — with the copy's stamp
/// at least as strict as `$5`. Inserts nothing when there is no live season.
pub(crate) const INSERT_DRAFT_FROM_LIVE_SQL: &str = concat!(
    "INSERT INTO training_plans (",
    plan_columns!(),
    ") SELECT $1, tenant_id, user_id, author_agent_id, goal_fact_id, goal_race_json, races_json, \
     strategy, phases_json, 'draft', NULL, source_conversation_id, $4, $4, flavour_json, \
     season_start, season_end, CASE WHEN $5 THEN $5 ELSE first_party_only END, $6, id, FALSE \
     FROM training_plans WHERE tenant_id = $2 AND user_id = $3 AND status = 'active' \
     RETURNING ",
    plan_columns!()
);

/// Re-lay the outline of the coach's open draft `$1`, in place: a draft is
/// the coach's workspace until it is published or discarded, and its id is
/// how a client tells one draft from the next.
pub(crate) const RELAY_DRAFT_OUTLINE_SQL: &str = concat!(
    "UPDATE training_plans SET author_agent_id = $4, goal_fact_id = $5, goal_race_json = $6, \
     races_json = $7, strategy = $8, phases_json = $9, source_conversation_id = $10, \
     flavour_json = $11, season_start = $12, season_end = $13, \
     first_party_only = CASE WHEN $14 THEN $14 ELSE first_party_only END, \
     outline_drafted = TRUE, updated_at = $15 \
     WHERE id = $1 AND tenant_id = $2 AND user_id = $3 AND status = 'draft' \
     RETURNING ",
    plan_columns!()
);

/// The athlete's live version of one week (`$3`): the row a draft week is
/// written over, when the live season has one.
pub(crate) const LIVE_WEEK_ID_SQL: &str = "SELECT w.id FROM training_plan_weeks w \
     JOIN training_plans p ON p.id = w.plan_id AND p.tenant_id = w.tenant_id \
     AND p.user_id = w.user_id AND p.status = 'active' \
     WHERE w.tenant_id = $1 AND w.user_id = $2 AND w.week_start = $3 AND w.status = 'active'";

/// The weeks of the coach's open draft `$3`, the draft's own version of each.
/// The parent must be `$4`'s open draft of this athlete, so neither another
/// coach's draft nor a live season's weeks come back.
pub(crate) const LIST_DRAFT_WEEKS_SQL: &str = concat!(
    "SELECT ",
    week_columns!(),
    " FROM training_plan_weeks \
     WHERE tenant_id = $1 AND user_id = $2 AND plan_id = $3 AND status = 'active' \
     AND EXISTS (SELECT 1 FROM training_plans p WHERE p.id = $3 AND p.tenant_id = $1 \
     AND p.user_id = $2 AND p.status = 'draft' AND p.drafted_by_user_id = $4) \
     ORDER BY week_start ASC"
);

/// The drafted weeks (`week_start`) of draft `$3` whose live version in the
/// season `$4` is not the one the coach drafted over: no version of the week
/// in the draft supersedes that live row. Only the first version of a week in
/// a draft supersedes a live row — later ones supersede the draft's own — so
/// this asks whether the live row is still the one the draft began the week
/// from.
pub(crate) const DRAFT_WEEK_CONFLICTS_SQL: &str = "SELECT d.week_start FROM training_plan_weeks d \
     JOIN training_plan_weeks live ON live.tenant_id = d.tenant_id AND live.user_id = d.user_id \
     AND live.week_start = d.week_start AND live.plan_id = $4 AND live.status = 'active' \
     WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.plan_id = $3 AND d.status = 'active' \
     AND NOT EXISTS (SELECT 1 FROM training_plan_weeks base WHERE base.tenant_id = d.tenant_id \
     AND base.user_id = d.user_id AND base.plan_id = d.plan_id \
     AND base.week_start = d.week_start AND base.supersedes_id = live.id) \
     ORDER BY d.week_start ASC";

/// When the coach last published a draft of this athlete's plan: a published
/// draft keeps its row, `status = 'published'`, stamped at the publish.
pub(crate) const LAST_PUBLISHED_DRAFT_SQL: &str = "SELECT updated_at FROM training_plans \
     WHERE tenant_id = $1 AND user_id = $2 AND drafted_by_user_id = $3 AND status = 'published' \
     ORDER BY updated_at DESC LIMIT 1";

/// Mark a draft published, as its content goes live in the same transaction.
pub(crate) const PUBLISHED_DRAFT_SQL: &str = "UPDATE training_plans \
     SET status = 'published', updated_at = $4 \
     WHERE id = $1 AND tenant_id = $2 AND user_id = $3 AND status = 'draft'";

/// Throw the coach's open draft away, returning its id.
pub(crate) const DISCARD_DRAFT_SQL: &str = "UPDATE training_plans \
     SET status = 'discarded', updated_at = $4 \
     WHERE tenant_id = $1 AND user_id = $2 AND drafted_by_user_id = $3 AND status = 'draft' \
     RETURNING id";

/// The outline a draft save carries, in the shape the outline encoder reads.
pub(crate) const fn draft_outline_params<'a>(
    params: &SaveDraftParams<'a>,
    outline: &PlanOutlineInput<'a>,
) -> SaveTrainingPlanParams<'a> {
    SaveTrainingPlanParams {
        tenant_id: params.owner.tenant_id,
        user_id: params.owner.user_id,
        author: params.author,
        goal_fact_id: params.goal_fact_id,
        goal_race: outline.goal_race,
        races: outline.races,
        strategy: outline.strategy,
        flavour: outline.flavour,
        season_start: outline.season_start,
        season_end: outline.season_end,
        phases: outline.phases,
        source_conversation_id: outline.source_conversation_id,
        transport_policy: params.transport_policy,
    }
}

/// The error an insert opening a draft meets. The one-draft index means a
/// concurrent save opened the coach's draft while this one ran: the
/// transaction rolls back and nothing is written.
pub(crate) fn draft_insert_error(e: &sqlx::Error) -> AppError {
    if e.as_database_error()
        .is_some_and(DatabaseError::is_unique_violation)
    {
        AppError::new(
            ErrorCode::ResourceAlreadyExists,
            "The draft of this athlete's plan changed while this save ran, so nothing was \
             saved — read it again with get_training_plan and save again.",
        )
    } else {
        AppError::database(format!("open training plan draft: {e}"))
    }
}

/// Emit the in-transaction helpers the draft half of the repository runs on,
/// for one backend's sqlx driver. Invoked by each backend shell beside
/// `impl_training_plan_repository!`, whose trait methods call them by name,
/// so the shell must `use` every const, helper and type the body names.
macro_rules! impl_training_plan_draft_helpers {
    ($db:ty) => {
        /// Write one week into a coach's open draft, `target.plan_id`. It supersedes
        /// the draft's own version of its `week_start`; the first version supersedes
        /// the athlete's live week instead — never flipping it, only naming it — which
        /// is the version the coach drafted over and the one publishing compares the
        /// live week against ([`DRAFT_WEEK_CONFLICTS_SQL`]).
        async fn supersede_and_insert_draft_week(
            conn: &mut <$db as sqlx::Database>::Connection,
            target: WeekTarget<'_>,
            week: &PlanWeekInput<'_>,
        ) -> AppResult<PlanWeek> {
            let v = week_insert_values(week.days)?;
            let previous: Option<String> = sqlx::query_scalar(SUPERSEDE_ACTIVE_WEEK_SQL)
                .bind(v.now)
                .bind(target.tenant_id)
                .bind(target.user_id)
                .bind(target.plan_id)
                .bind(week.week_start)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("supersede draft week: {e}")))?;
            let superseded = match previous {
                Some(id) => Some(id),
                None => sqlx::query_scalar(LIVE_WEEK_ID_SQL)
                    .bind(target.tenant_id)
                    .bind(target.user_id)
                    .bind(week.week_start)
                    .fetch_optional(&mut *conn)
                    .await
                    .map_err(|e| AppError::database(format!("live week under draft: {e}")))?,
            };
            insert_week(conn, target, week, superseded, v).await
        }

        /// The coach's open draft, read off a connection.
        async fn read_draft(
            conn: &mut <$db as sqlx::Database>::Connection,
            owner: DraftOwner<'_>,
        ) -> AppResult<Option<TrainingPlanRow>> {
            sqlx::query(DRAFT_PLAN_SQL)
                .bind(owner.tenant_id)
                .bind(owner.user_id)
                .bind(owner.coach_user_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("read training plan draft: {e}")))?
                .map(|row| plan_row(&row))
                .transpose()
        }

        /// The coach's open draft, claimed on an in-transaction connection
        /// ([`CLAIM_DRAFT_SQL`]); `saved_at` stamps it as saved.
        async fn claim_draft(
            conn: &mut <$db as sqlx::Database>::Connection,
            owner: DraftOwner<'_>,
            saved_at: Option<i64>,
        ) -> AppResult<Option<TrainingPlanRow>> {
            sqlx::query(CLAIM_DRAFT_SQL)
                .bind(owner.tenant_id)
                .bind(owner.user_id)
                .bind(owner.coach_user_id)
                .bind(saved_at)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("claim training plan draft: {e}")))?
                .map(|row| plan_row(&row))
                .transpose()
        }

        /// The athlete's live season read off an in-transaction connection.
        async fn live_plan(
            conn: &mut <$db as sqlx::Database>::Connection,
            tenant_id: &str,
            user_id: &str,
        ) -> AppResult<Option<TrainingPlan>> {
            sqlx::query(ACTIVE_PLAN_SQL)
                .bind(tenant_id)
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("read live plan: {e}")))?
                .map(|row| plan_row(&row).and_then(training_plan_from_row))
                .transpose()
        }

        /// Open the coach's draft with the outline the save carries. The draft
        /// records the live season it began from, and an outline that states no
        /// race calendar carries the live one, as an outline re-save does.
        async fn open_draft_with_outline(
            conn: &mut <$db as sqlx::Database>::Connection,
            params: &SaveDraftParams<'_>,
            outline: &PlanOutlineInput<'_>,
            now: i64,
        ) -> AppResult<TrainingPlanRow> {
            let owner = params.owner;
            let live = live_plan(conn, owner.tenant_id, owner.user_id).await?;
            let stp = draft_outline_params(params, outline);
            let v = plan_insert_values(&stp)?;
            let races_json = match (v.races_json, live.as_ref()) {
                (Some(json), _) => json,
                (None, Some(live)) => serde_json::to_string(&live.races)
                    .map_err(|e| AppError::internal(format!("serialize live races: {e}")))?,
                (None, None) => "[]".to_owned(),
            };
            let transport_policy = TransportPolicy::strictest_of([
                params.transport_policy,
                live.as_ref()
                    .map_or(TransportPolicy::AnyTransport, |p| p.transport_policy),
            ]);
            let row = sqlx::query(INSERT_DRAFT_SQL)
                .bind(&v.id)
                .bind(owner.tenant_id)
                .bind(owner.user_id)
                .bind(&v.author_agent_id)
                .bind(params.goal_fact_id)
                .bind(&v.goal_race_json)
                .bind(&races_json)
                .bind(outline.strategy)
                .bind(&v.phases_json)
                .bind(outline.source_conversation_id)
                .bind(now)
                .bind(v.flavour_json.as_deref())
                .bind(outline.season_start)
                .bind(outline.season_end)
                .bind(transport_policy.is_first_party_only())
                .bind(owner.coach_user_id)
                .bind(live.as_ref().map(|p| p.id.as_str()))
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| draft_insert_error(&e))?;
            plan_row(&row)
        }

        /// Open the coach's draft over the live season, its outline copied
        /// verbatim. `None` when the athlete has no live season to copy.
        async fn open_draft_from_live(
            conn: &mut <$db as sqlx::Database>::Connection,
            params: &SaveDraftParams<'_>,
            now: i64,
        ) -> AppResult<Option<TrainingPlanRow>> {
            let owner = params.owner;
            sqlx::query(INSERT_DRAFT_FROM_LIVE_SQL)
                .bind(Uuid::new_v4().to_string())
                .bind(owner.tenant_id)
                .bind(owner.user_id)
                .bind(now)
                .bind(params.transport_policy.is_first_party_only())
                .bind(owner.coach_user_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| draft_insert_error(&e))?
                .map(|row| plan_row(&row))
                .transpose()
        }

        /// Re-lay the outline of the coach's open draft in place. An outline that
        /// states no race calendar keeps the one the draft holds.
        async fn relay_draft_outline(
            conn: &mut <$db as sqlx::Database>::Connection,
            params: &SaveDraftParams<'_>,
            draft: &TrainingPlanRow,
            outline: &PlanOutlineInput<'_>,
            now: i64,
        ) -> AppResult<TrainingPlanRow> {
            let owner = params.owner;
            let stp = draft_outline_params(params, outline);
            let v = plan_insert_values(&stp)?;
            let races_json = v.races_json.unwrap_or_else(|| draft.races_json.clone());
            let row = sqlx::query(RELAY_DRAFT_OUTLINE_SQL)
                .bind(&draft.id)
                .bind(owner.tenant_id)
                .bind(owner.user_id)
                .bind(&v.author_agent_id)
                .bind(params.goal_fact_id)
                .bind(&v.goal_race_json)
                .bind(&races_json)
                .bind(outline.strategy)
                .bind(&v.phases_json)
                .bind(outline.source_conversation_id)
                .bind(v.flavour_json.as_deref())
                .bind(outline.season_start)
                .bind(outline.season_end)
                .bind(params.transport_policy.is_first_party_only())
                .bind(now)
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("re-lay draft outline: {e}")))?;
            plan_row(&row)
        }

        /// What publishing `draft` over the live season `live` would overwrite.
        async fn draft_conflicts(
            conn: &mut <$db as sqlx::Database>::Connection,
            owner: DraftOwner<'_>,
            draft: &PlanDraft,
            live: Option<&TrainingPlan>,
        ) -> AppResult<DraftConflicts> {
            let season_relaid = draft.base_plan_id.as_deref() != live.map(|p| p.id.as_str());
            let weeks = match live {
                Some(live) => sqlx::query_scalar(DRAFT_WEEK_CONFLICTS_SQL)
                    .bind(owner.tenant_id)
                    .bind(owner.user_id)
                    .bind(&draft.plan.id)
                    .bind(&live.id)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(|e| AppError::database(format!("draft week conflicts: {e}")))?,
                None => Vec::new(),
            };
            Ok(DraftConflicts {
                season_relaid,
                weeks,
            })
        }

        /// The weeks of the coach's open draft, read off a connection.
        async fn draft_weeks(
            conn: &mut <$db as sqlx::Database>::Connection,
            owner: DraftOwner<'_>,
            draft_id: &str,
        ) -> AppResult<Vec<PlanWeek>> {
            sqlx::query(LIST_DRAFT_WEEKS_SQL)
                .bind(owner.tenant_id)
                .bind(owner.user_id)
                .bind(draft_id)
                .bind(owner.coach_user_id)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("list draft weeks: {e}")))?
                .iter()
                .map(|row| week_row(row).and_then(plan_week_from_row))
                .collect()
        }

        /// Replay the claimed `draft` onto the athlete's live season `live`, which
        /// the caller claimed in the same transaction: a re-laid outline (or a
        /// draft whose season is gone) supersedes the live season, any author's,
        /// and carries its weeks; otherwise the weeks go on the live season as it
        /// stands. Each drafted week then supersedes the live week of its
        /// `week_start`, authored by whoever wrote it into the draft.
        async fn replay_draft(
            conn: &mut <$db as sqlx::Database>::Connection,
            params: &PublishDraftParams<'_>,
            draft: &PlanDraft,
            live: Option<&TrainingPlan>,
        ) -> AppResult<PublishedDraft> {
            let owner = params.owner;
            let draft_plan = &draft.plan;
            let outline_published = draft.outline_drafted || live.is_none();
            let (plan, superseded_plan_id) = if outline_published {
                let stp = SaveTrainingPlanParams {
                    tenant_id: owner.tenant_id,
                    user_id: owner.user_id,
                    author: PlanAuthor::from_agent(draft_plan.author_agent_id.as_deref()),
                    goal_fact_id: params.goal_fact_id.or(draft_plan.goal_fact_id.as_deref()),
                    goal_race: &draft_plan.goal_race,
                    races: Some(&draft_plan.races),
                    strategy: &draft_plan.strategy,
                    flavour: draft_plan.flavour.as_ref(),
                    season_start: draft_plan.season_start.as_deref(),
                    season_end: draft_plan.season_end.as_deref(),
                    phases: &draft_plan.phases,
                    source_conversation_id: draft_plan.source_conversation_id.as_deref(),
                    transport_policy: draft_plan.transport_policy,
                };
                let plan = supersede_and_insert_plan(&mut *conn, &stp, true).await?;
                let superseded = plan.supersedes_id.clone();
                (plan, superseded)
            } else {
                // `outline_published` is false only with a live season in hand.
                let plan = live.cloned().ok_or_else(|| {
                    AppError::internal("a weeks-only publish reached the replay with no season")
                })?;
                (plan, None)
            };
            let drafted = draft_weeks(&mut *conn, owner, &draft_plan.id).await?;
            let mut weeks = Vec::with_capacity(drafted.len());
            for week in &drafted {
                let target = WeekTarget {
                    tenant_id: owner.tenant_id,
                    user_id: owner.user_id,
                    plan_id: &plan.id,
                    author: PlanAuthor::from_agent(week.author_agent_id.as_deref()),
                };
                let input = PlanWeekInput {
                    week_start: &week.week_start,
                    focus: &week.focus,
                    days: &week.days,
                    adjustment_reason: &week.adjustment_reason,
                    phase_index: week.phase_index,
                };
                weeks.push(supersede_and_insert_week(&mut *conn, target, &input).await?);
            }
            sqlx::query(PUBLISHED_DRAFT_SQL)
                .bind(&draft_plan.id)
                .bind(owner.tenant_id)
                .bind(owner.user_id)
                .bind(Utc::now().timestamp())
                .execute(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("mark draft published: {e}")))?;
            Ok(PublishedDraft {
                draft_id: draft_plan.id.clone(),
                plan,
                weeks,
                superseded_plan_id,
                outline_published,
            })
        }

        /// [`TrainingPlanRepository::save_draft_bundle`] on `pool`.
        async fn save_draft_on(
            pool: &sqlx::Pool<$db>,
            params: &SaveDraftParams<'_>,
        ) -> AppResult<SavedDraftBundle> {
            let now = Utc::now().timestamp();
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| AppError::database(format!("begin draft tx: {e}")))?;

            // The coach's open draft, claimed so that a concurrent save or
            // publish of it waits; opened here when there is none.
            let claimed = claim_draft(&mut tx, params.owner, Some(now)).await?;
            let (row, opened) = match (claimed, params.outline.as_ref()) {
                (Some(row), None) => (row, false),
                (Some(row), Some(outline)) => (
                    relay_draft_outline(&mut tx, params, &row, outline, now).await?,
                    false,
                ),
                (None, Some(outline)) => (
                    open_draft_with_outline(&mut tx, params, outline, now).await?,
                    true,
                ),
                (None, None) => (
                    open_draft_from_live(&mut tx, params, now)
                        .await?
                        .ok_or_else(|| {
                            AppError::invalid_input(
                                "the athlete has no plan to draft weeks over — draft an outline \
                             first",
                            )
                        })?,
                    true,
                ),
            };
            let draft = plan_draft_from_row(row)?;

            let mut weeks = Vec::with_capacity(params.weeks.len());
            for week in params.weeks {
                let target = WeekTarget {
                    tenant_id: params.owner.tenant_id,
                    user_id: params.owner.user_id,
                    plan_id: &draft.plan.id,
                    author: params.author,
                };
                weeks.push(supersede_and_insert_draft_week(&mut tx, target, week).await?);
            }

            tx.commit()
                .await
                .map_err(|e| AppError::database(format!("commit draft tx: {e}")))?;
            Ok(SavedDraftBundle {
                draft,
                weeks,
                opened,
            })
        }

        /// [`TrainingPlanRepository::preview_draft_publish`] on `pool`.
        async fn preview_draft_on(
            pool: &sqlx::Pool<$db>,
            owner: DraftOwner<'_>,
        ) -> AppResult<Option<DraftPreview>> {
            let mut conn = pool
                .acquire()
                .await
                .map_err(|e| AppError::database(format!("acquire for draft preview: {e}")))?;
            let Some(row) = read_draft(&mut conn, owner).await? else {
                return Ok(None);
            };
            let draft = plan_draft_from_row(row)?;
            let live = live_plan(&mut conn, owner.tenant_id, owner.user_id).await?;
            let conflicts = draft_conflicts(&mut conn, owner, &draft, live.as_ref()).await?;
            Ok(Some(DraftPreview { draft, conflicts }))
        }

        /// [`TrainingPlanRepository::publish_draft`] on `pool`.
        async fn publish_draft_on(
            pool: &sqlx::Pool<$db>,
            params: &PublishDraftParams<'_>,
        ) -> AppResult<DraftPublish> {
            let owner = params.owner;
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| AppError::database(format!("begin publish tx: {e}")))?;

            // The draft, then the live season, each claimed: neither can move
            // between the conflict check below and the writes that follow it.
            let Some(row) = claim_draft(&mut tx, owner, None).await? else {
                return Ok(DraftPublish::NoDraft);
            };
            let draft = plan_draft_from_row(row)?;
            let live = claim_active_plan(
                &mut tx,
                owner.tenant_id,
                owner.user_id,
                draft.plan.transport_policy,
            )
            .await?;

            let conflicts = draft_conflicts(&mut tx, owner, &draft, live.as_ref()).await?;
            if !conflicts.is_empty() && !params.overwrite {
                // Dropping the transaction rolls back the two claims: a refused
                // publish writes nothing.
                return Ok(DraftPublish::Conflicted(DraftPreview { draft, conflicts }));
            }

            let published = replay_draft(&mut tx, params, &draft, live.as_ref()).await?;
            tx.commit()
                .await
                .map_err(|e| AppError::database(format!("commit publish tx: {e}")))?;
            Ok(DraftPublish::Published(published))
        }
    };
}
pub(crate) use impl_training_plan_draft_helpers;
