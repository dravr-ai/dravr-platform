// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A running turn as the steps it has taken, the time it has run and the line it folds into when its reply lands
// ABOUTME: One reducer and one label table for both chat UIs; each client only paints what it returns

import type { AwaitingReply, Message, TurnProgress } from '@pierre/shared-types';

import { replyLandedSince } from './conversation';
import { countKey, type CountKeys } from './plural';
import type { TranslatableText } from './text';

/** How long a turn runs before its indicator shows the time it has taken. */
export const TURN_ELAPSED_AFTER_MS = 10_000;

/** How long a turn runs before its indicator says the athlete may leave. */
export const TURN_SAFE_TO_LEAVE_AFTER_MS = 45_000;

/** How often a client re-reads a thread the server says is still being answered. */
export const AWAITING_REPLY_POLL_MS = 4_000;

/** The ACP tool-call states that mean the call has not finished yet. */
const TOOL_RUNNING_STATES: ReadonlySet<string> = new Set(['Pending', 'InProgress']);

/**
 * The state a started tool call ends in when it never ran after all — the
 * server's safety check blocked it, or parked it for the athlete to confirm.
 * Not an ACP state: the platform's own tool loops report it.
 */
const TOOL_NOT_RUN = 'NotRun';

/** Stage status the server reports as a stage is entered. */
const STAGE_STARTED = 'started';

/** The stage in which the server reads the question and builds the prompt. */
const PROMPT_ASSEMBLY_STAGE = 'prompt_assembly';

/** Catalogue keys of the steps a turn takes, by what the step does. */
export const TURN_STEP_KEYS = {
  thinking: 'chat.turn.step.thinking',
  readingQuestion: 'chat.turn.step.readingQuestion',
  analysing: 'chat.turn.step.analysing',
  writing: 'chat.turn.step.writing',
  lookingUp: 'chat.turn.step.lookingUp',
  readActivities: 'chat.turn.step.readActivities',
  readMemberActivities: 'chat.turn.step.readMemberActivities',
  readHistory: 'chat.turn.step.readHistory',
  readRecovery: 'chat.turn.step.readRecovery',
  readProfile: 'chat.turn.step.readProfile',
  readGroup: 'chat.turn.step.readGroup',
  choosePlanApproach: 'chat.turn.step.choosePlanApproach',
  readPlan: 'chat.turn.step.readPlan',
  pickWorkouts: 'chat.turn.step.pickWorkouts',
  savePlan: 'chat.turn.step.savePlan',
  checkWeather: 'chat.turn.step.checkWeather',
  nutrition: 'chat.turn.step.nutrition',
  predict: 'chat.turn.step.predict',
  updateCalendar: 'chat.turn.step.updateCalendar',
  syncData: 'chat.turn.step.syncData',
  checkConnections: 'chat.turn.step.checkConnections',
  manageConnection: 'chat.turn.step.manageConnection',
  updateProfile: 'chat.turn.step.updateProfile',
  setGoal: 'chat.turn.step.setGoal',
  reviewGoals: 'chat.turn.step.reviewGoals',
  updateNotes: 'chat.turn.step.updateNotes',
  reviewNotes: 'chat.turn.step.reviewNotes',
  recommend: 'chat.turn.step.recommend',
  findRoutes: 'chat.turn.step.findRoutes',
  recipes: 'chat.turn.step.recipes',
  lookUpFood: 'chat.turn.step.lookUpFood',
  chooseMobility: 'chat.turn.step.chooseMobility',
  browseAgents: 'chat.turn.step.browseAgents',
  addAgent: 'chat.turn.step.addAgent',
  linkMember: 'chat.turn.step.linkMember',
} as const;

/** The other words the indicator and the folded line are made of. */
export const TURN_TRAIL_KEYS = {
  elapsedSeconds: 'chat.turn.elapsedSeconds',
  elapsedMinutes: 'chat.turn.elapsedMinutes',
  elapsedHours: 'chat.turn.elapsedHours',
  safeToLeave: 'chat.turn.safeToLeave',
  foldedOne: 'chat.turn.foldedOne',
  foldedN: 'chat.turn.foldedN',
  foldedNoSteps: 'chat.turn.foldedNoSteps',
  showSteps: 'chat.turn.showSteps',
  hideSteps: 'chat.turn.hideSteps',
  stepDone: 'chat.turn.stepDone',
  stepsLabel: 'chat.turn.stepsLabel',
  tabTitle: 'chat.turn.tabTitle',
} as const;

/** The folded line's step count, singular and plural; both forms interpolate `{{count}}`. */
export const TURN_FOLDED_KEYS: CountKeys = {
  one: TURN_TRAIL_KEYS.foldedOne,
  many: TURN_TRAIL_KEYS.foldedN,
};

type StepKind = keyof typeof TURN_STEP_KEYS;

/**
 * What each tool the agent calls is doing, in the athlete's words — the one
 * table both clients read.
 *
 * A tool's own name (`get_group_member_activities`) is plumbing: the trail
 * says what the step does for the athlete. It covers every tool a chat turn
 * can call (the server's `chat_callable_schemas`); a name not listed here
 * reads as the generic "looking things up" — never as its name.
 */
const TOOL_STEP: Readonly<Record<string, StepKind>> = {
  get_activities: 'readActivities',
  get_activity_intelligence: 'readActivities',
  analyze_activity: 'readActivities',
  compare_activities: 'readActivities',
  extract_activity_streams: 'readActivities',
  get_heart_rate_series: 'readActivities',
  calculate_metrics: 'readActivities',
  export_intervals: 'readActivities',
  export_routes: 'readActivities',
  export_latest_snapshot: 'readActivities',
  get_group_member_activities: 'readMemberActivities',
  get_training_history: 'readHistory',
  compute_training_history: 'readHistory',
  get_stats: 'readHistory',
  analyze_performance_trends: 'readHistory',
  analyze_training_load: 'readHistory',
  calculate_fitness_score: 'readHistory',
  detect_patterns: 'readHistory',
  get_recovery_metrics: 'readRecovery',
  calculate_recovery_score: 'readRecovery',
  get_sleep_sessions: 'readRecovery',
  analyze_sleep_quality: 'readRecovery',
  get_health_snapshots: 'readRecovery',
  track_sleep_trends: 'readRecovery',
  optimize_sleep_schedule: 'readRecovery',
  get_athlete: 'readProfile',
  estimate_lactate_thresholds: 'readProfile',
  export_dossier: 'readProfile',
  set_physiology: 'updateProfile',
  import_provider_thresholds: 'updateProfile',
  get_roster_overview: 'readGroup',
  get_provider_roster: 'readGroup',
  propose_athlete_link: 'linkMember',
  recommend_plan_flavour: 'choosePlanApproach',
  get_training_plan: 'readPlan',
  get_planned_workouts: 'readPlan',
  list_workout_templates: 'pickWorkouts',
  save_training_plan: 'savePlan',
  publish_training_plan: 'savePlan',
  prescribe_workout: 'updateCalendar',
  push_training_plan: 'updateCalendar',
  withdraw_prescribed_workout: 'updateCalendar',
  get_weather_forecast: 'checkWeather',
  analyze_weather_impact: 'checkWeather',
  discover_routes: 'findRoutes',
  compose_route: 'findRoutes',
  calculate_daily_nutrition: 'nutrition',
  analyze_meal_nutrition: 'nutrition',
  get_nutrient_timing: 'nutrition',
  search_food: 'lookUpFood',
  get_food_details: 'lookUpFood',
  get_recipe: 'recipes',
  get_recipe_constraints: 'recipes',
  list_recipes: 'recipes',
  search_recipes: 'recipes',
  validate_recipe: 'recipes',
  save_recipe: 'recipes',
  delete_recipe: 'recipes',
  list_stretching_exercises: 'chooseMobility',
  get_stretching_exercise: 'chooseMobility',
  suggest_stretches_for_activity: 'chooseMobility',
  list_yoga_poses: 'chooseMobility',
  get_yoga_pose: 'chooseMobility',
  suggest_yoga_sequence: 'chooseMobility',
  predict_performance: 'predict',
  analyze_goal_feasibility: 'predict',
  estimate_vo2max: 'predict',
  set_goal: 'setGoal',
  suggest_goals: 'reviewGoals',
  track_progress: 'reviewGoals',
  generate_recommendations: 'recommend',
  suggest_rest_day: 'recommend',
  remember_fact: 'updateNotes',
  agent_note_add: 'updateNotes',
  agent_followup_schedule: 'updateNotes',
  commitment_create: 'updateNotes',
  commitment_cancel: 'updateNotes',
  recall_user_memory: 'reviewNotes',
  list_coaching_playbooks: 'reviewNotes',
  refresh_provider_data: 'syncData',
  get_connection_status: 'checkConnections',
  get_data_freshness: 'checkConnections',
  list_data_sources: 'checkConnections',
  connect_provider: 'manageConnection',
  disconnect_provider: 'manageConnection',
  browse_agent_store: 'browseAgents',
  search_agent_store: 'browseAgents',
  install_agent_from_store: 'addAgent',
};

/** Tool names longest first, so a qualified title matches its most specific tool. */
const KNOWN_TOOLS: readonly string[] = Object.keys(TOOL_STEP).sort((a, b) => b.length - a.length);

/**
 * The step a tool call is, from the title its progress frame carries.
 *
 * The title is the tool's name, sometimes qualified by the runner
 * (`pierre__get_training_plan`), so a known name anywhere in it counts.
 */
export function toolStepLabel(title: string): TranslatableText {
  const exact = TOOL_STEP[title];
  if (exact) return { key: TURN_STEP_KEYS[exact] };
  const lowered = title.toLowerCase();
  const known = KNOWN_TOOLS.find(name => lowered.includes(name));
  return { key: TURN_STEP_KEYS[known ? TOOL_STEP[known] : 'lookingUp'] };
}

/** One tool call the turn made. */
export interface TurnStep {
  /** The call's protocol id; unique within the turn. */
  id: string;
  /** What the step does, as a catalogue key. */
  label: TranslatableText;
  /** Whether the call has finished — completed, failed or cancelled alike. */
  done: boolean;
}

/** What the turn is doing between tool calls, or before the first. */
export type TurnPhase = 'readingQuestion' | 'thinking' | 'writing';

/** A turn in flight, or one that just ended, as its steps and its clock. */
export interface TurnTrail {
  /** When the turn started (epoch ms). */
  startedAt: number;
  /** The tool calls, in the order they started. */
  steps: TurnStep[];
  /** What the turn is doing when no tool call is running. */
  phase: TurnPhase;
  /** When the reply landed (epoch ms); `null` while the turn runs. */
  finishedAt: number | null;
}

/** Something that happened to a turn while it ran. */
export type TurnTrailEvent =
  /** A progress frame arrived on the turn's stream. */
  | { type: 'progress'; progress: TurnProgress }
  /** The reply's first words arrived: the agent is writing. */
  | { type: 'prose' }
  /**
   * The words streamed so far were taken back — they preceded a tool call,
   * or the reply is being written again: the agent is thinking once more.
   */
  | { type: 'retract' }
  /**
   * The turn ended at `at`. `toolsCalled` is the finished turn's own record
   * of the tools it ran (`telemetry.tools_called`), which may name calls no
   * progress frame announced — a runner that streams no tool frames.
   */
  | { type: 'finished'; at: number; toolsCalled?: readonly string[] };

/** A turn that started at `at` and has done nothing yet. */
export function startTurnTrail(at: number): TurnTrail {
  return { startedAt: at, steps: [], phase: 'thinking', finishedAt: null };
}

function applyProgress(trail: TurnTrail, progress: TurnProgress): TurnTrail {
  if (progress.kind === 'stage') {
    // Only the question-reading stage says something the athlete can follow;
    // every other stage is the model thinking, which is the default phase.
    if (progress.status !== STAGE_STARTED) return trail;
    const phase: TurnPhase = progress.title === PROMPT_ASSEMBLY_STAGE ? 'readingQuestion' : 'thinking';
    return phase === trail.phase ? trail : { ...trail, phase };
  }
  const index = trail.steps.findIndex(step => step.id === progress.id);
  if (progress.status === TOOL_NOT_RUN) {
    // A call that never ran is not a step the turn took: it leaves the trail
    // rather than being checked off.
    if (index === -1) return trail;
    return { ...trail, steps: trail.steps.filter(step => step.id !== progress.id) };
  }
  const done = !TOOL_RUNNING_STATES.has(progress.status);
  if (index === -1) {
    const step: TurnStep = { id: progress.id, label: toolStepLabel(progress.title), done };
    // A tool call means the question has been read; between calls the
    // model is thinking again.
    return { ...trail, steps: [...trail.steps, step], phase: 'thinking' };
  }
  const existing = trail.steps[index];
  if (existing.done === done) return trail;
  const steps = trail.steps.slice();
  steps[index] = { ...existing, done };
  return { ...trail, steps };
}

/** The tool calls a finished turn ran that no progress frame announced, as done steps. */
function unannouncedSteps(steps: readonly TurnStep[], toolsCalled: readonly string[]): TurnStep[] {
  if (toolsCalled.length <= steps.length) return [];
  return toolsCalled.slice(steps.length).map((tool, index) => ({
    id: `called-${steps.length + index}`,
    label: toolStepLabel(tool),
    done: true,
  }));
}

/**
 * The trail after `event`.
 *
 * A tool call is added when it starts and checked off when it ends, whatever
 * way it ended — unless it ends never having run, which takes it off the
 * trail. The phase between calls is thinking, never a reset to the first
 * line. The finished turn closes every step.
 */
export function reduceTurnTrail(trail: TurnTrail, event: TurnTrailEvent): TurnTrail {
  if (trail.finishedAt !== null) return trail;
  switch (event.type) {
    case 'progress':
      return applyProgress(trail, event.progress);
    case 'prose':
      return trail.phase === 'writing' ? trail : { ...trail, phase: 'writing' };
    case 'retract':
      return trail.phase === 'writing' ? { ...trail, phase: 'thinking' } : trail;
    case 'finished': {
      const closed = trail.steps.map(step => (step.done ? step : { ...step, done: true }));
      return {
        ...trail,
        steps: [...closed, ...unannouncedSteps(closed, event.toolsCalled ?? [])],
        finishedAt: Math.max(event.at, trail.startedAt),
      };
    }
  }
}

/**
 * One thread's trails: the running turn's, and those its replies fold into.
 *
 * `finished` is keyed by the assistant message each trail led to, and is kept
 * for the session only — the steps are not persisted, so a reload folds
 * nothing.
 */
export interface TurnTrails {
  /** The running turn's trail; `null` when no turn streams on this client. */
  live: TurnTrail | null;
  /** Finished trails, keyed by the reply they led to. */
  finished: ReadonlyMap<string, TurnTrail>;
}

/** What moves a thread's trails. Instants are passed in, so the reducer stays pure. */
export type TurnTrailsAction =
  | { type: 'begin'; at: number }
  | { type: 'event'; event: TurnTrailEvent }
  | { type: 'finish'; at: number; replyId: string | null; toolsCalled?: readonly string[] }
  | { type: 'reset' };

/** A thread with no turn running and nothing folded. */
export const EMPTY_TURN_TRAILS: TurnTrails = { live: null, finished: new Map() };

/**
 * Move a thread's trails — the one state machine both clients' hooks hold.
 *
 * An event or a finish with no live turn changes nothing. A finish with the
 * reply's id closes the trail and keeps it to fold above that reply; without
 * one (a failure, a stop with nothing written) the trail is dropped. `reset`
 * forgets every trail, for when another thread opens. An action that changes
 * nothing returns the same state, so a hook re-renders only on a change.
 */
export function reduceTurnTrails(state: TurnTrails, action: TurnTrailsAction): TurnTrails {
  switch (action.type) {
    case 'begin':
      return { ...state, live: startTurnTrail(action.at) };
    case 'event': {
      if (state.live === null) return state;
      const live = reduceTurnTrail(state.live, action.event);
      return live === state.live ? state : { ...state, live };
    }
    case 'finish': {
      if (state.live === null) return state;
      if (action.replyId === null) return { ...state, live: null };
      const closed = reduceTurnTrail(state.live, {
        type: 'finished',
        at: action.at,
        toolsCalled: action.toolsCalled,
      });
      return { live: null, finished: new Map(state.finished).set(action.replyId, closed) };
    }
    case 'reset':
      return state.live === null && state.finished.size === 0 ? state : EMPTY_TURN_TRAILS;
  }
}

/** The step running now: the newest unfinished tool call, if any. */
export function runningStep(trail: TurnTrail): TurnStep | null {
  for (let i = trail.steps.length - 1; i >= 0; i -= 1) {
    if (!trail.steps[i].done) return trail.steps[i];
  }
  return null;
}

/**
 * The line that names what the turn is doing now.
 *
 * A running tool names itself; otherwise the phase does, and once a tool has
 * finished, thinking reads as analysing what it brought back.
 */
export function turnCurrentLine(trail: TurnTrail): TranslatableText {
  const running = runningStep(trail);
  if (running) return running.label;
  if (trail.phase === 'writing') return { key: TURN_STEP_KEYS.writing };
  if (trail.phase === 'readingQuestion' && trail.steps.length === 0) {
    return { key: TURN_STEP_KEYS.readingQuestion };
  }
  return { key: trail.steps.length > 0 ? TURN_STEP_KEYS.analysing : TURN_STEP_KEYS.thinking };
}

/** The finished tool calls, oldest first — the checked part of the trail. */
export function doneSteps(trail: TurnTrail): TurnStep[] {
  return trail.steps.filter(step => step.done);
}

/** How long the turn has run at `now`, never negative (a server clock ahead of ours). */
export function turnElapsedMs(startedAt: number, now: number): number {
  return Math.max(0, now - startedAt);
}

/**
 * A duration as the indicator prints it — "14 s", "2 min 14 s", "1 h 05 min".
 *
 * Catalogue keys rather than `Intl` unit formatting, which the phone's
 * JavaScript runtime does not carry.
 */
export function formatTurnDuration(ms: number): TranslatableText {
  const totalSeconds = Math.floor(Math.max(0, ms) / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return { key: TURN_TRAIL_KEYS.elapsedHours, params: { hours, minutes } };
  if (minutes > 0) return { key: TURN_TRAIL_KEYS.elapsedMinutes, params: { minutes, seconds } };
  return { key: TURN_TRAIL_KEYS.elapsedSeconds, params: { seconds } };
}

/** What the live indicator shows besides its steps, at `elapsedMs` into the turn. */
export interface TurnIndicator {
  /** The time the turn has run, once it is long enough to be worth saying. */
  elapsed: TranslatableText | null;
  /** The note that the athlete may leave and find the reply here. */
  safeToLeave: TranslatableText | null;
}

/** The indicator's timer and note at `elapsedMs` into a turn. */
export function turnIndicator(elapsedMs: number): TurnIndicator {
  return {
    elapsed: elapsedMs >= TURN_ELAPSED_AFTER_MS ? formatTurnDuration(elapsedMs) : null,
    safeToLeave: elapsedMs >= TURN_SAFE_TO_LEAVE_AFTER_MS ? { key: TURN_TRAIL_KEYS.safeToLeave } : null,
  };
}

/** The one line a finished turn folds into above its reply. */
export interface FoldedTrail {
  /** The line's key; its `duration` param is the resolved {@link FoldedTrail.duration}. */
  line: TranslatableText;
  /** How long the turn took, to resolve and pass as the line's `duration`. */
  duration: TranslatableText;
  /** The steps the line expands into. */
  steps: TurnStep[];
}

/**
 * What a finished turn folds into, or `null` when it is not worth a line —
 * a quick answer that called no tool.
 *
 * A turn with no step reads `foldedNoSteps`; otherwise [`countKey`] picks the
 * step count's form in `language`.
 */
export function foldTurnTrail(trail: TurnTrail, language: string | undefined): FoldedTrail | null {
  if (trail.finishedAt === null) return null;
  const tookMs = trail.finishedAt - trail.startedAt;
  const steps = doneSteps(trail);
  if (steps.length === 0 && tookMs < TURN_ELAPSED_AFTER_MS) return null;
  const count = steps.length;
  const key = count === 0 ? TURN_TRAIL_KEYS.foldedNoSteps : countKey(count, language, TURN_FOLDED_KEYS);
  return { line: { key, params: { count } }, duration: formatTurnDuration(tookMs), steps };
}

/**
 * When the reply the server says it is still writing was asked for (epoch
 * ms), or `null` when nothing is awaited.
 *
 * The flag is the server's, but the client may already hold the answer — a
 * re-read that raced the reply, or a cache the live turn wrote — so a reply
 * after the question in `transcript` settles it whatever the flag says.
 */
export function awaitedSince(
  awaiting: AwaitingReply | null | undefined,
  transcript: readonly Pick<Message, 'id' | 'role'>[],
): number | null {
  if (!awaiting) return null;
  const questionAt = transcript.findIndex(row => row.id === awaiting.question_id);
  if (questionAt !== -1) {
    const heldBefore = new Set(transcript.slice(0, questionAt).map(row => row.id));
    if (replyLandedSince(transcript, heldBefore)) return null;
  }
  const askedAt = Date.parse(awaiting.asked_at);
  return Number.isNaN(askedAt) ? null : askedAt;
}
