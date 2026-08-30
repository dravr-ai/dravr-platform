// ABOUTME: TypeScript shape of the structured-workout plan emitted by builder coaches.
// ABOUTME: Mirrors schemas/structured-workout.schema.json; consumed by the web/mobile plan cards.

/** Inclusive [min, max] pair (pace/power %, RPE, lactate mmol, etc.). */
export type WorkoutRange = [number, number];

export interface WorkoutPlanWindow {
  start: string;
  end: string;
}

export interface WorkoutCompliance {
  polarization_index?: number | null;
  z1_pct?: number;
  z2_pct?: number;
  z3_pct?: number;
  weekly_tss_target?: number;
  heat_session_count?: number;
  active_minutes_total?: number;
  passive_minutes_total?: number;
}

export type WorkoutBlockType =
  | 'warmup'
  | 'steady'
  | 'interval'
  | 'tempo'
  | 'recovery'
  | 'cooldown';

export interface WorkoutBlock {
  type: WorkoutBlockType;
  reps: number;
  work_min: number;
  rest_min: number;
  zone: number;
  target_pct_ftp_or_pace: WorkoutRange;
  rpe: WorkoutRange;
  target_hr_pct_max?: WorkoutRange;
}

/**
 * Per-session fuelling target. Mirrors `$defs.FuelingProtocol` in
 * structured-workout.schema.json, where all three rates are required.
 * `sodium_mg_per_h` is an estimated sweat loss, never a required intake.
 */
export interface FuelingProtocol {
  carbs_g_per_h: number;
  fluid_ml_per_h: number;
  sodium_mg_per_h: number;
  carb_source?: string;
}

/**
 * Fluid-only variant carried by heat sessions. Mirrors
 * `$defs.FluidProtocol`, which omits the carbohydrate rate.
 */
export interface FluidProtocol {
  fluid_ml_per_h: number;
  sodium_mg_per_h: number;
}

export interface WorkoutSession {
  name: string;
  duration_min: number;
  intensity_factor: number;
  tss_estimate: number;
  blocks: WorkoutBlock[];
  lactate_target_mmol?: WorkoutRange;
  pace_power_target_pct?: WorkoutRange;
  hr_cap_bpm_over_lt?: number;
  fueling_protocol?: FuelingProtocol;
  fluid_protocol?: FluidProtocol;
}

export type WorkoutDayName = 'Mon' | 'Tue' | 'Wed' | 'Thu' | 'Fri' | 'Sat' | 'Sun';

export interface WorkoutDay {
  day: WorkoutDayName;
  session: WorkoutSession | null;
  am_pm_split?: boolean;
  pm_session?: WorkoutSession;
}

export interface WorkoutWeek {
  week_index: number;
  ctl_target?: number;
  days: WorkoutDay[];
  gut_training_progression?: string[];
}

/**
 * A structured training plan. Required fields mirror the schema's `required`
 * list; coach-specific fields (lactate, taper, heat, ultra) are optional.
 */
export interface WorkoutPlan {
  plan_window: WorkoutPlanWindow;
  rationale: string;
  compliance: WorkoutCompliance;
  weeks: WorkoutWeek[];
  evidence_refs: string[];
  lactate_targets_mmol?: { LT1: WorkoutRange; LT2: WorkoutRange };
  lactate_fallback_mode?: 'lactate' | 'pace_power' | 'hr_cap';
  easy_volume_floor_pct?: number;
}

/**
 * Parse a `Message.structured_content` JSON string into a `WorkoutPlan`.
 * Returns `null` when the payload is absent or not a well-formed plan object,
 * so callers fall back to rendering the raw text.
 */
export function parseWorkoutPlan(structuredContent: string | undefined): WorkoutPlan | null {
  if (!structuredContent) {
    return null;
  }
  try {
    const parsed = JSON.parse(structuredContent) as Partial<WorkoutPlan>;
    if (
      parsed &&
      typeof parsed === 'object' &&
      parsed.plan_window &&
      Array.isArray(parsed.weeks) &&
      parsed.weeks.length > 0
    ) {
      return parsed as WorkoutPlan;
    }
    return null;
  } catch {
    return null;
  }
}
