// ABOUTME: Wire shapes of a coach's plan draft for one athlete — the draft state, the subject plan read, publish and its conflict
// ABOUTME: Mirrors the subject-plan routes of carnet#1069/#1070; each parser rejects a body of any other shape instead of rendering half of it

import { instantMs } from './instant.js';
import { isWorkoutPlan, type WorkoutPlan } from './workout-plan.js';

/** Why publishing a draft week would overwrite the athlete's own work. */
export type DraftConflictReason = 'athlete_changed_week';

/**
 * One week the coach drafted whose live version the athlete's agent saved
 * after the draft began. Publishing it would overwrite that save.
 */
export interface DraftConflict {
  /** The week's Monday, `YYYY-MM-DD`. */
  week_start: string;
  reason: DraftConflictReason;
}

/**
 * Where the coach's open draft stands: which draft it is, when it was last
 * saved, and what publishing it would collide with.
 */
export interface DraftState {
  draft_id: string;
  /** When the coach last saved into the draft (RFC 3339). */
  updated_at: string;
  conflicts: DraftConflict[];
  /** The athlete's live season was re-laid after the draft began. */
  season_relaid: boolean;
}

/** The athlete a coach's plan thread, and its draft, are about. */
export interface PlanSubject {
  user_id: string;
  display_name: string;
}

/**
 * `GET /api/chat/conversations/{id}/subject-plan`: the subject's live plan
 * and the coach's draft over it, as the coach reads them.
 */
export interface SubjectPlanResponse {
  athlete: PlanSubject;
  /** The athlete's today, `YYYY-MM-DD`, the day both plans were projected for. */
  today: string;
  live: WorkoutPlan | null;
  draft: WorkoutPlan | null;
  draft_state: DraftState | null;
}

/** `POST …/subject-plan/publish` body. */
export interface PublishSubjectPlanRequest {
  /** Publish over the weeks the athlete changed, and over a re-laid season. */
  overwrite_athlete_changes: boolean;
}

/** `POST …/subject-plan/publish` answered 200: the draft is the athlete's live plan. */
export interface PublishedSubjectPlan {
  plan_id: string;
  /** The season the published plan replaced, when there was one. */
  superseded_plan_id: string | null;
  /** Whether the athlete was sent the published notice. */
  athlete_notified: boolean;
}

/** The `error_code` of a publish refused because the athlete's plan moved under the draft. */
export const PLAN_PUBLISH_CONFLICT = 'plan_publish_conflict';

/** `POST …/subject-plan/publish` answered 409: what publishing would overwrite. */
export interface PlanPublishConflict {
  error_code: typeof PLAN_PUBLISH_CONFLICT;
  conflicts: DraftConflict[];
  season_relaid: boolean;
}

/**
 * What a publish came to. A conflict is an answer the coach acts on — publish
 * anyway, or keep drafting — not a failure, so it is a result rather than a
 * thrown error.
 */
export type PublishSubjectPlanResult =
  | { status: 'published'; published: PublishedSubjectPlan }
  | { status: 'conflict'; conflict: PlanPublishConflict };

const CIVIL_DATE = /^\d{4}-\d{2}-\d{2}$/;

const CONFLICT_REASONS: readonly DraftConflictReason[] = ['athlete_changed_week'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseConflicts(value: unknown): DraftConflict[] | null {
  if (!Array.isArray(value)) return null;
  const conflicts: DraftConflict[] = [];
  for (const entry of value) {
    if (!isRecord(entry) || typeof entry.week_start !== 'string' || !CIVIL_DATE.test(entry.week_start)) {
      return null;
    }
    const reason = CONFLICT_REASONS.find((known) => known === entry.reason);
    if (reason === undefined) return null;
    conflicts.push({ week_start: entry.week_start, reason });
  }
  return conflicts;
}

/** A draft state, or `null` when `value` is not one. */
export function parseDraftState(value: unknown): DraftState | null {
  if (
    !isRecord(value) ||
    typeof value.draft_id !== 'string' ||
    value.draft_id.length === 0 ||
    typeof value.updated_at !== 'string' ||
    Number.isNaN(instantMs(value.updated_at)) ||
    typeof value.season_relaid !== 'boolean'
  ) {
    return null;
  }
  const conflicts = parseConflicts(value.conflicts);
  if (conflicts === null) return null;
  return {
    draft_id: value.draft_id,
    updated_at: value.updated_at,
    conflicts,
    season_relaid: value.season_relaid,
  };
}

/** A plan subject, or `null` when `value` is not one. */
export function parsePlanSubject(value: unknown): PlanSubject | null {
  if (
    !isRecord(value) ||
    typeof value.user_id !== 'string' ||
    value.user_id.length === 0 ||
    typeof value.display_name !== 'string'
  ) {
    return null;
  }
  return { user_id: value.user_id, display_name: value.display_name };
}

/** A plan that is a card or none (null or omitted); `undefined` for anything else. */
function nullablePlan(value: unknown): WorkoutPlan | null | undefined {
  if (value === null || value === undefined) return null;
  return isWorkoutPlan(value) ? value : undefined;
}

/** The subject-plan read, or `null` when the body breaks its contract. */
export function parseSubjectPlanResponse(body: unknown): SubjectPlanResponse | null {
  if (!isRecord(body) || typeof body.today !== 'string' || !CIVIL_DATE.test(body.today)) {
    return null;
  }
  const athlete = parsePlanSubject(body.athlete);
  const live = nullablePlan(body.live);
  const draft = nullablePlan(body.draft);
  const stateOmitted = body.draft_state === null || body.draft_state === undefined;
  const draftState = stateOmitted ? null : parseDraftState(body.draft_state);
  if (
    athlete === null ||
    live === undefined ||
    draft === undefined ||
    (!stateOmitted && draftState === null)
  ) {
    return null;
  }
  return { athlete, today: body.today, live, draft, draft_state: draftState };
}

/** A publish's 200 body, or `null` when it breaks its contract. */
export function parsePublishedSubjectPlan(body: unknown): PublishedSubjectPlan | null {
  if (
    !isRecord(body) ||
    typeof body.plan_id !== 'string' ||
    !(body.superseded_plan_id === null || typeof body.superseded_plan_id === 'string') ||
    typeof body.athlete_notified !== 'boolean'
  ) {
    return null;
  }
  return {
    plan_id: body.plan_id,
    superseded_plan_id: body.superseded_plan_id,
    athlete_notified: body.athlete_notified,
  };
}

/** A publish's 409 conflict body, or `null` when it is not one. */
export function parsePlanPublishConflict(body: unknown): PlanPublishConflict | null {
  if (!isRecord(body) || body.error_code !== PLAN_PUBLISH_CONFLICT || typeof body.season_relaid !== 'boolean') {
    return null;
  }
  const conflicts = parseConflicts(body.conflicts);
  if (conflicts === null) return null;
  return { error_code: PLAN_PUBLISH_CONFLICT, conflicts, season_relaid: body.season_relaid };
}
