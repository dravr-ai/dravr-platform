// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's hand-written session structure on both clients — steps, sets of steps under one ×N, time or distance, and targets (carnet#1727)
// ABOUTME: Pure list operations over PlanStep[]; the sets are the providers' repeat grouping made visible, the targets the server's grammar composed

import { clockDuration } from '@pierre/chat-utils';
import type { PlanStep } from '@pierre/shared-types';

/** A step a coach adds: five minutes, unnamed and with no target, until they write it. */
const NEW_STEP: PlanStep = { label: '', duration_seconds: 300, target_zone: '' };

/** The repetitions a new set starts on, and the bounds the set's count keeps. */
export const SERIES_DEFAULT_REPEAT = 5;
export const SERIES_MIN_REPEAT = 1;
export const SERIES_MAX_REPEAT = 50;

/**
 * Whether two adjacent steps repeat together, the way the calendar push
 * groups them: both repeat more than once, the same number of times, and in
 * the same numbered set when either names one.
 */
function sameSet(a: PlanStep, b: PlanStep): boolean {
  const ra = a.repeat ?? 1;
  const rb = b.repeat ?? 1;
  if (ra <= 1 || rb <= 1 || ra !== rb) return false;
  if (a.repeat_group !== undefined || b.repeat_group !== undefined) return a.repeat_group === b.repeat_group;
  return true;
}

/** One block of a session: a lone step, or a set of steps done `repeat` times in a row. */
export type StepBlock =
  | { kind: 'step'; index: number }
  | { kind: 'series'; start: number; end: number; repeat: number };

/** The session's steps as blocks, in order. A set's `end` is exclusive. */
export function stepBlocks(steps: readonly PlanStep[]): StepBlock[] {
  const blocks: StepBlock[] = [];
  let at = 0;
  while (at < steps.length) {
    const step = steps[at] as PlanStep;
    const repeat = step.repeat ?? 1;
    if (repeat <= 1) {
      blocks.push({ kind: 'step', index: at });
      at += 1;
      continue;
    }
    let end = at + 1;
    while (end < steps.length && sameSet(steps[end - 1] as PlanStep, steps[end] as PlanStep)) end += 1;
    blocks.push({ kind: 'series', start: at, end, repeat });
    at = end;
  }
  return blocks;
}

/** Whether the block holds the step at `index`. */
function holds(block: StepBlock, index: number): boolean {
  return block.kind === 'step' ? block.index === index : index >= block.start && index < block.end;
}

/** How many steps a block spans. */
function blockSize(block: StepBlock): number {
  return block.kind === 'step' ? 1 : block.end - block.start;
}

/** The block holding the step at `index`. */
function blockOf(steps: readonly PlanStep[], index: number): StepBlock | undefined {
  return stepBlocks(steps).find((block) => holds(block, index));
}

/**
 * The steps with every set numbered in order (1, 2, …) and every lone step
 * stripped of its repeat: the grouping on screen frozen, so an edit to one
 * set can never merge it into its neighbour.
 */
export function numberedSteps(steps: readonly PlanStep[]): PlanStep[] {
  const out: PlanStep[] = steps.map((step) => ({ ...step }));
  let group = 0;
  for (const block of stepBlocks(steps)) {
    if (block.kind === 'step') {
      const step = out[block.index] as PlanStep;
      delete step.repeat;
      delete step.repeat_group;
      continue;
    }
    group += 1;
    for (let i = block.start; i < block.end; i += 1) {
      const step = out[i] as PlanStep;
      step.repeat = block.repeat;
      step.repeat_group = group;
    }
  }
  return out;
}

/** A lone step added at the end. */
export function appendStep(steps: readonly PlanStep[]): PlanStep[] {
  return [...numberedSteps(steps), { ...NEW_STEP }];
}

/** What a new set's two steps start as: their names and targets, in the coach's words. */
export interface SeriesDefaults {
  work: { label: string; target: string };
  recovery: { label: string; target: string };
}

/** A set of an effort and a recovery added at the end, done `SERIES_DEFAULT_REPEAT` times. */
export function appendSeries(steps: readonly PlanStep[], defaults: SeriesDefaults): PlanStep[] {
  const numbered = numberedSteps(steps);
  const group = numbered.reduce((max, step) => Math.max(max, step.repeat_group ?? 0), 0) + 1;
  const set = { repeat: SERIES_DEFAULT_REPEAT, repeat_group: group };
  return [
    ...numbered,
    { label: defaults.work.label, duration_seconds: 180, target_zone: defaults.work.target, ...set },
    { label: defaults.recovery.label, duration_seconds: 120, target_zone: defaults.recovery.target, ...set },
  ];
}

/** A step added at the end of the set that starts at `start`, repeating with it. */
export function appendToSeries(steps: readonly PlanStep[], start: number): PlanStep[] {
  const numbered = numberedSteps(steps);
  const block = blockOf(numbered, start);
  if (block?.kind !== 'series') return numbered;
  const first = numbered[block.start] as PlanStep;
  const added: PlanStep = { ...NEW_STEP, repeat: first.repeat, repeat_group: first.repeat_group };
  return [...numbered.slice(0, block.end), added, ...numbered.slice(block.end)];
}

/** The set that starts at `start` done `repeat` times, held inside the set's bounds; at one, its steps stand alone. */
export function setSeriesRepeat(steps: readonly PlanStep[], start: number, repeat: number): PlanStep[] {
  const numbered = numberedSteps(steps);
  const block = blockOf(numbered, start);
  if (block?.kind !== 'series') return numbered;
  const count = Math.min(SERIES_MAX_REPEAT, Math.max(SERIES_MIN_REPEAT, Math.round(repeat)));
  for (let i = block.start; i < block.end; i += 1) {
    const step = numbered[i] as PlanStep;
    if (count <= 1) {
      delete step.repeat;
      delete step.repeat_group;
    } else {
      step.repeat = count;
    }
  }
  return numbered;
}

/** A copy of the step at `index` placed right after it, in its set when it has one. */
export function duplicateStep(steps: readonly PlanStep[], index: number): PlanStep[] {
  const numbered = numberedSteps(steps);
  const step = numbered[index];
  if (!step) return numbered;
  return [...numbered.slice(0, index + 1), { ...step }, ...numbered.slice(index + 1)];
}

/** The step at `index` removed. */
export function removeStepAt(steps: readonly PlanStep[], index: number): PlanStep[] {
  return numberedSteps(steps).filter((_, at) => at !== index);
}

/**
 * The step at `index` moved by `delta`: inside its set when it shares one
 * with other steps, else past the whole block beside it, so a move never
 * splits a set or lands a lone step inside one.
 */
export function moveStepAt(steps: readonly PlanStep[], index: number, delta: -1 | 1): PlanStep[] {
  const numbered = numberedSteps(steps);
  const blocks = stepBlocks(numbered);
  const at = blocks.findIndex((block) => holds(block, index));
  const block = blocks[at];
  if (!block) return numbered;
  if (block.kind === 'series' && blockSize(block) > 1) {
    const to = index + delta;
    if (to < block.start || to >= block.end) return numbered;
    const out = [...numbered];
    [out[index], out[to]] = [out[to] as PlanStep, out[index] as PlanStep];
    return out;
  }
  return moveBlockAt(numbered, at, delta);
}

/**
 * Where `moveStepAt` lands the step at `index` when it moves it by `delta`,
 * or `null` when that move would change nothing: the clients disable such a
 * move, and keep the keyboard on the step at its new place.
 */
export function stepMoveTarget(steps: readonly PlanStep[], index: number, delta: -1 | 1): number | null {
  const blocks = stepBlocks(steps);
  const at = blocks.findIndex((block) => holds(block, index));
  const block = blocks[at];
  if (!block) return null;
  if (block.kind === 'series' && blockSize(block) > 1) {
    const to = index + delta;
    return to >= block.start && to < block.end ? to : null;
  }
  const beside = blocks[at + delta];
  return beside ? index + delta * blockSize(beside) : null;
}

/** The block at `blockIndex` (in `stepBlocks` order) moved past the block beside it. */
export function moveBlockAt(steps: readonly PlanStep[], blockIndex: number, delta: -1 | 1): PlanStep[] {
  const numbered = numberedSteps(steps);
  const blocks = stepBlocks(numbered);
  const to = blockIndex + delta;
  if (blockIndex < 0 || blockIndex >= blocks.length || to < 0 || to >= blocks.length) return numbered;
  const slices = blocks.map((block) =>
    block.kind === 'step' ? numbered.slice(block.index, block.index + 1) : numbered.slice(block.start, block.end),
  );
  [slices[blockIndex], slices[to]] = [slices[to] as PlanStep[], slices[blockIndex] as PlanStep[]];
  return numberedSteps(slices.flat());
}

/** The step at `index` with `patch` applied; a `repeat` or set is changed through the set, never here. */
export function patchStepAt(steps: readonly PlanStep[], index: number, patch: Partial<PlanStep>): PlanStep[] {
  return steps.map((step, at) => (at === index ? { ...step, ...patch } : step));
}

/** How a step is measured: by its time, or by a distance with an estimated time. */
export type StepMeasure = 'time' | 'distance';

/** The measure a step is written in. */
export function stepMeasure(step: PlanStep): StepMeasure {
  return step.distance_meters === undefined ? 'time' : 'distance';
}

/**
 * The step at `index` measured by `measure`. A distance step keeps a time:
 * the server totals a session from its steps' times, so the coach states an
 * estimate beside the distance. A distance not yet written reads as 0 until
 * the coach states it, and the save waits for it.
 */
export function setStepMeasure(steps: readonly PlanStep[], index: number, measure: StepMeasure): PlanStep[] {
  return steps.map((step, at) => {
    if (at !== index) return step;
    if (measure === 'distance') return { ...step, distance_meters: step.distance_meters ?? 0 };
    const next = { ...step };
    delete next.distance_meters;
    return next;
  });
}

/** Every step's time times its repeats: what the server writes as the session's length. */
export function stepsTotalSeconds(steps: readonly PlanStep[]): number {
  return steps.reduce((sum, step) => sum + Math.max(0, step.duration_seconds) * Math.max(1, step.repeat ?? 1), 0);
}

/** A step's duration as the field shows it: `m:ss` (`90` → `1:30`, `600` → `10:00`). */
export function formatStepDuration(seconds: number): string {
  return clockDuration(seconds);
}

/**
 * A step's duration as typed, in seconds: `m:ss`, `:ss`, or bare minutes
 * (`1:30` → 90, `:20` → 20, `10` → 600). `undefined` for anything else,
 * seconds of 60 or more included.
 */
export function parseStepDuration(text: string): number | undefined {
  const trimmed = text.trim();
  const match = /^(\d*):(\d{1,2})$/.exec(trimmed);
  if (match) {
    const seconds = Number(match[2]);
    if (seconds >= 60) return undefined;
    return Number(match[1] || '0') * 60 + seconds;
  }
  return /^\d+$/.test(trimmed) ? Number(trimmed) * 60 : undefined;
}

/** The threshold a percent target is of, as the editor offers it. */
export type TargetAnchor = 'ftp' | 'threshold_hr' | 'threshold_pace';

/** The anchors in the editor's order. */
export const TARGET_ANCHORS: readonly TargetAnchor[] = ['ftp', 'threshold_hr', 'threshold_pace'];

/** How the server's grammar spells each anchor (`88-93% FTP`, `95-100% threshold HR`, `90-95% threshold pace`). */
const ANCHOR_WIRE: Record<TargetAnchor, string> = {
  ftp: 'FTP',
  threshold_hr: 'threshold HR',
  threshold_pace: 'threshold pace',
};

/** The highest zone the grammar reads (`Z1`–`Z7`). */
export const MAX_TARGET_ZONE = 7;

/** A step's target as the editor reads it: a zone, a percent band, an RPE band, or text as written. */
export type StepTarget =
  | { kind: 'none' }
  | { kind: 'zone'; zone: number }
  | { kind: 'percent'; low: number; high: number; anchor: TargetAnchor | null }
  | { kind: 'rpe'; low: number; high: number }
  | { kind: 'typed'; text: string };

const ZONE_RE = /^z([1-7])$/i;
const PERCENT_RE = /^(\d{1,3})(?:\s*-\s*(\d{1,3}))?\s*%\s*(ftp|threshold hr|threshold pace)?$/i;
const RPE_RE = /^rpe\s*(\d{1,2})(?:\s*-\s*(\d{1,2}))?$/i;

/**
 * A target as the chips show it. Only the spellings the chips themselves
 * write are read back as a chip; anything else (`sweet spot`, `tempo`, an
 * agent's `easy`) stays text as written, and the server says on save whether
 * it resolves — the grammar is the kernel's, never re-derived here.
 */
export function parseTarget(text: string): StepTarget {
  const trimmed = text.trim();
  if (trimmed === '') return { kind: 'none' };
  const zone = ZONE_RE.exec(trimmed);
  if (zone) return { kind: 'zone', zone: Number(zone[1]) };
  const percent = PERCENT_RE.exec(trimmed);
  if (percent) {
    const low = Number(percent[1]);
    const high = percent[2] === undefined ? low : Number(percent[2]);
    const word = percent[3]?.toLowerCase();
    const anchor = TARGET_ANCHORS.find((key) => ANCHOR_WIRE[key].toLowerCase() === word) ?? null;
    return { kind: 'percent', low, high, anchor };
  }
  const rpe = RPE_RE.exec(trimmed);
  if (rpe) {
    const low = Number(rpe[1]);
    return { kind: 'rpe', low, high: rpe[2] === undefined ? low : Number(rpe[2]) };
  }
  return { kind: 'typed', text: trimmed };
}

/** A zone target: `Z4`. */
export function zoneTarget(zone: number): string {
  return `Z${Math.min(MAX_TARGET_ZONE, Math.max(1, Math.round(zone)))}`;
}

/** A percent band of an anchor: `88-93% FTP`, `75% threshold HR`; a bare `75%` with no anchor. */
export function percentTarget(low: number, high: number, anchor: TargetAnchor | null): string {
  const a = Math.max(1, Math.round(Math.min(low, high)));
  const b = Math.max(1, Math.round(Math.max(low, high)));
  const band = a === b ? `${a}%` : `${a}-${b}%`;
  return anchor ? `${band} ${ANCHOR_WIRE[anchor]}` : band;
}

/** The RPE scale a band is written on: 1 to 10. */
const RPE_MIN = 1;
const RPE_MAX = 10;

/** An RPE band on the 1–10 scale: `RPE 7`, `RPE 6-7`. */
export function rpeTarget(low: number, high: number = low): string {
  const clamp = (value: number) => Math.min(RPE_MAX, Math.max(RPE_MIN, Math.round(value)));
  const a = clamp(Math.min(low, high));
  const b = clamp(Math.max(low, high));
  return a === b ? `RPE ${a}` : `RPE ${a}-${b}`;
}

/** The anchor a percent target starts on in a sport: power on the bike, pace elsewhere. */
export function defaultAnchor(sport: string | undefined): TargetAnchor {
  return sport === 'ride' ? 'ftp' : 'threshold_pace';
}

/**
 * The span the steps' targets cover, for the header line: `Z1–Z4`, `RPE 2–8`,
 * or one value. `null` when a target is missing, written as text, or the
 * targets mix kinds.
 */
export function targetSpan(steps: readonly PlanStep[]): string | null {
  if (steps.length === 0) return null;
  const parsed = steps.map((step) => parseTarget(step.target_zone));
  if (parsed.every((target) => target.kind === 'zone')) {
    const zones = parsed.map((target) => (target.kind === 'zone' ? target.zone : 0));
    const min = Math.min(...zones);
    const max = Math.max(...zones);
    return min === max ? `Z${min}` : `Z${min}–Z${max}`;
  }
  if (parsed.every((target) => target.kind === 'rpe')) {
    const lows = parsed.map((target) => (target.kind === 'rpe' ? target.low : 0));
    const highs = parsed.map((target) => (target.kind === 'rpe' ? target.high : 0));
    const min = Math.min(...lows);
    const max = Math.max(...highs);
    return min === max ? `RPE ${min}` : `RPE ${min}–${max}`;
  }
  return null;
}

/**
 * How high a bar is drawn for a target the chips do not write (`tempo`,
 * `sweet spot`, an agent's `easy`): one neutral height. Which zone a named
 * target reads as is the kernel's grammar (dravr-cageux), never a table
 * copied here, so the picture does not guess at it.
 */
const TYPED_LEVEL = 0.3;

/** The lowest a bar is drawn, so a step with no target or an easy one still shows. */
const LEVEL_FLOOR = 0.15;

/** The percent of a threshold drawn at full height. */
const PERCENT_AT_TOP = 120;

/** How high a step's bar is drawn, 0–1: the picture of its target, never a number the athlete reads. */
export function targetLevel(text: string): number {
  const target = parseTarget(text);
  switch (target.kind) {
    case 'zone':
      return target.zone / MAX_TARGET_ZONE;
    case 'rpe':
      return (target.low + target.high) / (2 * RPE_MAX);
    case 'percent':
      return Math.min(1, Math.max(LEVEL_FLOOR, (target.low + target.high) / (2 * PERCENT_AT_TOP)));
    case 'typed':
      return TYPED_LEVEL;
    default:
      return LEVEL_FLOOR;
  }
}

/** A bar at or above this level is drawn in the accent on both clients: the session's efforts stand out from its easy parts. */
export const PROFILE_EFFORT_LEVEL = 0.55;

/** One bar of the session's profile: a step, or one repetition of it inside a set. */
export interface ProfileBar {
  /** The step's index in the session's steps. */
  index: number;
  seconds: number;
  /** 0–1, from its target. */
  level: number;
  /** A second or later repetition of a set: drawn quieter, and not a stop for the keyboard. */
  repeated: boolean;
}

/** One block of the profile: a lone step's bar, or a set's bars across all its repetitions. */
export type ProfileBlock =
  | { kind: 'step'; bar: ProfileBar }
  | { kind: 'series'; start: number; repeat: number; bars: ProfileBar[] };

/** The session's profile, block by block, every repetition of a set drawn in turn. */
export function profileBlocks(steps: readonly PlanStep[]): ProfileBlock[] {
  const bar = (index: number, repeated: boolean): ProfileBar => {
    const step = steps[index] as PlanStep;
    return { index, seconds: Math.max(1, step.duration_seconds), level: targetLevel(step.target_zone), repeated };
  };
  return stepBlocks(steps).map((block) => {
    if (block.kind === 'step') return { kind: 'step', bar: bar(block.index, false) };
    const bars: ProfileBar[] = [];
    for (let round = 0; round < block.repeat; round += 1) {
      for (let i = block.start; i < block.end; i += 1) bars.push(bar(i, round > 0));
    }
    return { kind: 'series', start: block.start, repeat: block.repeat, bars };
  });
}

/** What a step still lacks before the session can be saved, and which step (1-based). */
export type StepGap = { kind: 'label' | 'duration' | 'distance' | 'target'; number: number };

/** The first gap in the steps, in step order, or `null` when every step is whole. */
export function firstStepGap(steps: readonly PlanStep[]): StepGap | null {
  for (const [at, step] of steps.entries()) {
    const number = at + 1;
    if (step.label.trim().length === 0) return { kind: 'label', number };
    if (!(step.duration_seconds > 0)) return { kind: 'duration', number };
    if (step.distance_meters !== undefined && !(step.distance_meters > 0)) return { kind: 'distance', number };
    if (step.target_zone.trim().length === 0) return { kind: 'target', number };
  }
  return null;
}

/** The steps as the save sends them: names and targets trimmed, sets numbered, lone steps without a repeat. */
export function stepsForSave(steps: readonly PlanStep[]): PlanStep[] {
  return numberedSteps(steps).map((step) => ({
    ...step,
    label: step.label.trim(),
    target_zone: step.target_zone.trim(),
  }));
}
