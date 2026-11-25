// ABOUTME: Wire shapes of a coach's plan draft for one athlete — the draft state, the subject plan read, publish, its conflict, the change history
// ABOUTME: Mirrors the subject-plan routes of carnet#1069/#1070/#1815; each parser rejects a body of any other shape instead of rendering half of it

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

/**
 * `GET /api/chat/athletes/{id}/plan?group_id=`: a coached athlete's live
 * plan as their coach reads it from Group info, read-only (carnet#1244).
 */
export interface AthletePlanResponse {
  athlete: PlanSubject;
  /** The athlete's today, `YYYY-MM-DD`, the day the plan was projected for. */
  today: string;
  live: WorkoutPlan | null;
}

/** `POST …/subject-plan/publish` body. */
export interface PublishSubjectPlanRequest {
  /** Publish over the weeks the athlete changed, and over a re-laid season. */
  overwrite_athlete_changes: boolean;
}

/** One calendar entry a push left alone or failed to write. */
export interface CalendarPushEntry {
  /** The entry's day, `YYYY-MM-DD`. */
  date: string;
  /** The entry's title. */
  name: string;
}

/** What a push wrote to the athlete's calendar. */
export interface CalendarPushReport {
  /** The calendar provider written, as the server names it (`intervals_icu`). */
  provider: string;
  created: number;
  updated: number;
  removed: number;
  /** Entries left alone because the athlete changed them on their calendar. */
  skipped: CalendarPushEntry[];
  /** Entries the calendar or the ledger refused. */
  failed: CalendarPushEntry[];
}

/**
 * What a publish did to the athlete's calendar (carnet#1492): the plan put on
 * their chosen calendar, or why nothing was written there. `reason` is the
 * refusing gate's code (`no_calendar`, `no_calendar_write_consent`,
 * `choose_calendar`, `push_failed`, …) and `message` the server's English
 * words for it.
 */
export type CalendarPushOutcome =
  | { status: 'pushed'; report: CalendarPushReport }
  | { status: 'not_pushed'; reason: string; message: string };

/** `POST …/subject-plan/publish` answered 200: the draft is the athlete's live plan. */
export interface PublishedSubjectPlan {
  plan_id: string;
  /** The season the published plan replaced, when there was one. */
  superseded_plan_id: string | null;
  /** Whether the athlete was sent the published notice. */
  athlete_notified: boolean;
  /**
   * Whether a line naming the days the publish changed was posted in the
   * group room the thread acts under (carnet#1249): `false` when the publish
   * changed no day, or the room is not the coach's and the athlete's alone.
   * Optional because a fixture or a cached answer written before the room
   * line carries none; the parser keeps it out rather than inventing one.
   */
  posted_in_room?: boolean;
  /**
   * The published plan put on the athlete's calendar, or why it was not.
   * Optional for the same reason as `posted_in_room`.
   */
  calendar_push?: CalendarPushOutcome;
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

/**
 * What kind of change was made to an athlete's plan in the coach's
 * workspace (carnet#1815): a session of the draft rewritten (`day_edit`),
 * added, removed or put back as it stood (`day_restore`); a date put back to
 * the live plan (`date_revert`); the coach's call made or taken off; weeks
 * or an outline the agent saved into the draft (`agent_draft`); the draft
 * published or discarded.
 */
export type PlanChangeKind =
  | 'day_edit'
  | 'day_add'
  | 'day_remove'
  | 'day_restore'
  | 'date_revert'
  | 'coach_call'
  | 'agent_draft'
  | 'publish'
  | 'discard';

/** Who made a change: the coach, or the thread's agent on the coach's word. */
export type PlanChangeSource = 'coach' | 'agent';

/** One change to the athlete's plan, as the coach's thread lists it. */
export interface PlanChangeEvent {
  kind: PlanChangeKind;
  source: PlanChangeSource;
  /** The dates it touched, `YYYY-MM-DD`, ascending; empty for a discard or an outline re-laid alone. */
  dates: string[];
  /** When it landed (RFC 3339): the `before` of the next, older page. */
  created_at: string;
}

/**
 * `GET /api/chat/conversations/{id}/subject-plan/history`: the changes the
 * coach and the thread's agent made to the athlete's plan, newest first.
 */
export interface SubjectPlanHistory {
  events: PlanChangeEvent[];
}

/** Which page of the history to read: before the oldest change the client holds, and how many. */
export interface SubjectPlanHistoryQuery {
  /** The `created_at` of the oldest change the client holds; absent for the newest. */
  before?: string;
  /** How many, `1..100`; the server answers 50 when absent. */
  limit?: number;
}

const CIVIL_DATE = /^\d{4}-\d{2}-\d{2}$/;

const PLAN_CHANGE_KINDS: readonly PlanChangeKind[] = [
  'day_edit',
  'day_add',
  'day_remove',
  'day_restore',
  'date_revert',
  'coach_call',
  'agent_draft',
  'publish',
  'discard',
];

const PLAN_CHANGE_SOURCES: readonly PlanChangeSource[] = ['coach', 'agent'];

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

/**
 * Whether a card's draft shows the coach's open draft as it stands: the same
 * draft, and no later save over it. A draft keeps its id across saves, so a
 * card from an earlier save shows a version the coach has since changed, and
 * publishing from it would publish something the card does not show.
 */
export function cardShowsOpenDraft(open: DraftState, card: DraftState): boolean {
  return open.draft_id === card.draft_id && instantMs(card.updated_at) >= instantMs(open.updated_at);
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

/** The coach's read-only athlete plan, or `null` when the body breaks its contract. */
export function parseAthletePlanResponse(body: unknown): AthletePlanResponse | null {
  if (!isRecord(body) || typeof body.today !== 'string' || !CIVIL_DATE.test(body.today)) {
    return null;
  }
  const athlete = parsePlanSubject(body.athlete);
  const live = nullablePlan(body.live);
  if (athlete === null || live === undefined) {
    return null;
  }
  return { athlete, today: body.today, live };
}

/** The entries of a push report's `skipped` or `failed`, or `null` when one breaks its shape. */
function pushEntries(value: unknown): CalendarPushEntry[] | null {
  if (!Array.isArray(value)) return null;
  const entries: CalendarPushEntry[] = [];
  for (const entry of value) {
    if (!isRecord(entry) || typeof entry.date !== 'string' || typeof entry.name !== 'string') return null;
    entries.push({ date: entry.date, name: entry.name });
  }
  return entries;
}

/** A publish's `calendar_push`, or `null` when it breaks its contract. */
function parseCalendarPushOutcome(value: unknown): CalendarPushOutcome | null {
  if (!isRecord(value)) return null;
  if (value.status === 'not_pushed') {
    return typeof value.reason === 'string' && typeof value.message === 'string'
      ? { status: 'not_pushed', reason: value.reason, message: value.message }
      : null;
  }
  if (value.status !== 'pushed' || !isRecord(value.report)) return null;
  const { report } = value;
  const skipped = pushEntries(report.skipped);
  const failed = pushEntries(report.failed);
  if (
    typeof report.provider !== 'string' ||
    typeof report.created !== 'number' ||
    typeof report.updated !== 'number' ||
    typeof report.removed !== 'number' ||
    skipped === null ||
    failed === null
  ) {
    return null;
  }
  return {
    status: 'pushed',
    report: {
      provider: report.provider,
      created: report.created,
      updated: report.updated,
      removed: report.removed,
      skipped,
      failed,
    },
  };
}

/** A publish's 200 body, or `null` when it breaks its contract. */
export function parsePublishedSubjectPlan(body: unknown): PublishedSubjectPlan | null {
  if (
    !isRecord(body) ||
    typeof body.plan_id !== 'string' ||
    !(body.superseded_plan_id === null || typeof body.superseded_plan_id === 'string') ||
    typeof body.athlete_notified !== 'boolean' ||
    !(body.posted_in_room === undefined || typeof body.posted_in_room === 'boolean')
  ) {
    return null;
  }
  const calendarPush = body.calendar_push === undefined ? undefined : parseCalendarPushOutcome(body.calendar_push);
  if (calendarPush === null) return null;
  return {
    plan_id: body.plan_id,
    superseded_plan_id: body.superseded_plan_id,
    athlete_notified: body.athlete_notified,
    ...(typeof body.posted_in_room === 'boolean' ? { posted_in_room: body.posted_in_room } : {}),
    ...(calendarPush === undefined ? {} : { calendar_push: calendarPush }),
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

/** One change of the history, or `null` when it breaks its contract. */
function parsePlanChangeEvent(value: unknown): PlanChangeEvent | null {
  if (!isRecord(value) || typeof value.created_at !== 'string' || Number.isNaN(instantMs(value.created_at))) {
    return null;
  }
  const kind = PLAN_CHANGE_KINDS.find((known) => known === value.kind);
  const source = PLAN_CHANGE_SOURCES.find((known) => known === value.source);
  const { dates } = value;
  if (
    kind === undefined ||
    source === undefined ||
    !Array.isArray(dates) ||
    !dates.every((date): date is string => typeof date === 'string' && CIVIL_DATE.test(date))
  ) {
    return null;
  }
  return { kind, source, dates: [...dates], created_at: value.created_at };
}

/** The subject plan's change history, or `null` when the body breaks its contract. */
export function parseSubjectPlanHistory(body: unknown): SubjectPlanHistory | null {
  if (!isRecord(body) || !Array.isArray(body.events)) return null;
  const events: PlanChangeEvent[] = [];
  for (const entry of body.events) {
    const event = parsePlanChangeEvent(entry);
    if (event === null) return null;
    events.push(event);
  }
  return { events };
}
