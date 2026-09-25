// ABOUTME: TypeScript shape of the workout_plan block — the saved plan projected for a card, never a document the model wrote.
// ABOUTME: Mirrors pierre_services::plan_card::PlanCard; consumed by the web and mobile plan cards.

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

export interface PlanDay {
  /** `YYYY-MM-DD`. */
  date: string;
  sport: string;
  workout: string;
  duration_min?: number;
  intensity: string;
  rest: boolean;
  steps?: PlanStep[];
  fueling?: PlanFueling;
  template_slug?: string;
  template_source?: PlanTemplateSource;
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
