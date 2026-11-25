// ABOUTME: TypeScript shape of the workout_plan block — the saved plan projected for a card, never a document the model wrote.
// ABOUTME: Mirrors pierre_services::plan_card::PlanCard; consumed by the web and mobile plan cards.

import type { SessionParams, SessionSport } from './session-editor.js';

/** The phase kinds the periodization kernel knows. */
export type PlanPhaseKind =
  | 'prep'
  | 'base'
  | 'build'
  | 'specialty'
  | 'peak'
  | 'taper'
  | 'race'
  | 'transition'
  | 'recovery';

/** Who chose the plan's flavour. */
export type PlanSelectedBy = 'rule' | 'coach' | 'athlete';

/** Which authorship tier a day's template resolved in when the day was saved. */
export type PlanTemplateSource = 'package' | 'catalogue' | 'athlete';

export interface PlanGoalRace {
  name: string;
  /** `YYYY-MM-DD`. */
  date: string;
  discipline: string;
  priority: 'A' | 'B' | 'C';
}

export interface PlanFlavour {
  /** Catalogue id. */
  id: string;
  /** The plain-words label in the athlete's locale, or the id when none exists. */
  label: string;
  selected_by: PlanSelectedBy;
}

/** One phase on the season timeline. */
export interface PlanPhase {
  kind: PlanPhaseKind;
  /** First day, `YYYY-MM-DD`. */
  start: string;
  /** Day after the last day, `YYYY-MM-DD`. */
  end?: string;
  weeks: number;
  purpose: string;
  intent: string;
  target_hours?: number;
  hard_sessions_max?: number;
  /** True for the phase covering the athlete's today. */
  current: boolean;
}

/** One step of a session's structure — the one vocabulary the calendar push writes too. */
export interface PlanStep {
  label: string;
  duration_seconds: number;
  distance_meters?: number;
  target_zone: string;
  repeat?: number;
  /**
   * Which set the step belongs to, numbered in step order, when the producer
   * numbers its sets — keeps apart two adjacent sets with the same `repeat`.
   */
  repeat_group?: number;
  note?: string;
}

export interface PlanFueling {
  carbs_g_per_h: number;
  fluid_ml_per_h: number;
  sodium_mg_per_h?: number;
  /** Carbohydrate source when the rate depends on it, e.g. "glucose:fructose 1:0.8". */
  carb_source?: string;
}

/**
 * In place of `fueling`, for a day that stores a protocol while a medical
 * flag is on the athlete's file: the athlete's clinician sets these amounts.
 */
export interface PlanFuelingWithheld {
  reason: 'medical_flag';
  set_by: 'clinician';
  /** The server's plain-words statement; clients show their own localized line. */
  note: string;
}

export interface PlanDay {
  /** `YYYY-MM-DD`. */
  date: string;
  /**
   * Where the entry sits among its date's entries, from 0, a rest entry
   * included: a date may hold several sessions, in the order they are done
   * (carnet#1716). Absent on a card written before it, which reads as 0.
   */
  position?: number;
  sport: string;
  workout: string;
  duration_min?: number;
  intensity: string;
  rest: boolean;
  steps?: PlanStep[];
  fueling?: PlanFueling;
  fueling_withheld?: PlanFuelingWithheld;
  template_slug?: string;
  template_source?: PlanTemplateSource;
  /** The family preset the day resolved to, when its template is a family (carnet#1515). */
  template_preset?: string;
  /** The values the day states on its family preset, by key (carnet#1515). */
  template_params?: SessionParams;
  /** The day's sport as a family preset reads it, when it is one of them (carnet#1515). */
  family_sport?: SessionSport;
  /**
   * The warning the card marks the day with when the envelope check found the
   * session outside its family's evidence, in the saving turn's language: what
   * a client prints when it has no words for the warning. The reply that saved
   * it carried the notice saying why. Absent otherwise.
   */
  outside_evidence?: string;
  /**
   * True when the envelope check found the session outside its family's
   * evidence: the client words the warning in its reader's language, since
   * `outside_evidence` is in the language of the turn that saved the card.
   */
  outside?: boolean;
  /**
   * Where the session's evidence sits when it is weaker than a trial in this
   * sport without a qualifier: one chip. Absent otherwise.
   */
  evidence?: PlanDayEvidence;
  /** "Counts as <purpose>", in the saving turn's language, when an edit relabelled the session. */
  counts_as?: string;
  /** The purpose that edit relabelled the session to, worded by the client (`messaging.envelope.purpose.<purpose>`). */
  counts_as_purpose?: string;
  /**
   * The session the coach's edits replaced, when the coach edited this day
   * of their draft from the plan thread (carnet#1714). Only a draft carries it.
   */
  edited?: PlanDayEdit;
  /** True when the coach added this session to the date in their draft (carnet#1716). */
  added?: boolean;
}

/** The session a coach's edits replaced on a day of their draft. */
export interface PlanDayEdit {
  /** The replaced session's workout, as the day wrote it. */
  workout: string;
  /** Whether the replaced day was a rest day. */
  rest: boolean;
}

/** The one chip a day's evidence earns, by name: a client words it as `messaging.evidence.<kind>`. */
export const PLAN_EVIDENCE_KINDS = [
  'convention',
  'transposed',
  'acute_only',
  'uncontrolled_only',
  'acute_uncontrolled',
] as const;
export type PlanEvidenceKind = (typeof PLAN_EVIDENCE_KINDS)[number];

/** Why a chip cites no study, by name: a client words it as `messaging.evidence.<kind>`. */
export const PLAN_EVIDENCE_NOTE_KINDS = ['no_basis_convention', 'no_basis_transposed'] as const;
export type PlanEvidenceNoteKind = (typeof PLAN_EVIDENCE_NOTE_KINDS)[number];

/** The evidence chip of a plan day. */
export interface PlanDayEvidence {
  /** The chip by name, worded in the reader's language; absent on a card saved before it travelled. */
  kind?: PlanEvidenceKind;
  /** The chip in the language of the turn that saved the card: what a client prints without `kind`. */
  label: string;
  /** The studies the evidence rests on, as cited; absent when the badge cites none. */
  basis?: string;
  /** What the chip opens on when it cites no study, in the saving turn's language. */
  note?: string;
  /** `note` by name, worded in the reader's language; absent when `note` is. */
  note_kind?: PlanEvidenceNoteKind;
}

export interface PlanWeek {
  /** Monday, `YYYY-MM-DD`. */
  week_start: string;
  focus: string;
  phase_index?: number;
  /** True when the athlete's today falls in this week. */
  current: boolean;
  days: PlanDay[];
}

/** The card: the season and the fortnight the athlete acts on. */
export interface WorkoutPlan {
  goal_race: PlanGoalRace;
  /** The other races on the calendar, in the order the outline stores them. */
  races?: PlanGoalRace[];
  season_start?: string;
  season_end?: string;
  flavour?: PlanFlavour;
  phases: PlanPhase[];
  current_phase_index?: number;
  weeks: PlanWeek[];
  /** Future weeks the plan holds beyond `weeks`. */
  weeks_deferred: number;
}

/**
 * Whether a decoded value has the shape of a card: a named goal race, and
 * phase and week arrays. The one check both the chat block and the Home read
 * make, so the two cannot disagree about what counts as a plan.
 */
export function isWorkoutPlan(value: unknown): value is WorkoutPlan {
  const candidate = value as Partial<WorkoutPlan> | null;
  return (
    typeof candidate === 'object' &&
    candidate !== null &&
    typeof candidate.goal_race === 'object' &&
    candidate.goal_race !== null &&
    typeof candidate.goal_race.name === 'string' &&
    Array.isArray(candidate.phases) &&
    Array.isArray(candidate.weeks)
  );
}

/**
 * Parse a `workout_plan` block's `plan` JSON into a `WorkoutPlan`.
 * Returns `null` when the payload is absent or not a well-formed card, so
 * callers render nothing rather than a broken card.
 */
export function parseWorkoutPlan(planJson: string | undefined): WorkoutPlan | null {
  if (!planJson) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(planJson);
    return isWorkoutPlan(parsed) ? parsed : null;
  } catch {
    return null;
  }
}
