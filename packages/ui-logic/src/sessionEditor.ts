// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's day editor on both clients — the day's own session edited by hand, or a family build with its live preview (carnet#1515, #1714)
// ABOUTME: The kernel derives every number on the server; this only holds what the coach wrote or picked and lays out what came back

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { ChatApi } from '@pierre/api-client';
import { refusalReason } from './apiError';
import {
  SESSION_PARAM_KEYS,
  SESSION_SPORTS,
  type FamilyCatalogueResponse,
  type AddedSession,
  type FamilyPreset,
  type HandDayRequest,
  type PlanDay,
  type PlanStep,
  type SessionBuild,
  type SessionDerived,
  type SessionParamKey,
  type SessionParamRange,
  type SessionExpect,
  type SessionPreview,
  type SessionSport,
  type SubjectPlanResponse,
  type WorkoutFamily,
} from '@pierre/shared-types';

/** The editor's i18n keys (contremaitre `strings/`, `plan.editor`). */
export const SESSION_EDITOR_KEYS = {
  edit: 'plan.editor.edit',
  editSession: 'plan.editor.editSession',
  editTitle: 'plan.editor.editTitle',
  addSession: 'plan.editor.addSession',
  addTitle: 'plan.editor.addTitle',
  removeSession: 'plan.editor.removeSession',
  removeTitle: 'plan.editor.removeTitle',
  removeBody: 'plan.editor.removeBody',
  removeConfirm: 'plan.editor.removeConfirm',
  removed: 'plan.editor.removed',
  sessionMoved: 'plan.editor.sessionMoved',
  dateFull: 'plan.editor.dateFull',
  removeFailed: 'plan.editor.removeFailed',
  added: 'plan.editor.added',
  restoreNoteAdded: 'plan.editor.restoreNoteAdded',
  modeSession: 'plan.editor.modeSession',
  modeFamily: 'plan.editor.modeFamily',
  restDay: 'plan.editor.restDay',
  workout: 'plan.editor.workout',
  duration: 'plan.editor.duration',
  intensity: 'plan.editor.intensity',
  stepLabel: 'plan.editor.stepLabel',
  stepDuration: 'plan.editor.stepDuration',
  stepTarget: 'plan.editor.stepTarget',
  stepRepeat: 'plan.editor.stepRepeat',
  addStep: 'plan.editor.addStep',
  removeStep: 'plan.editor.removeStep',
  moveStepUp: 'plan.editor.moveStepUp',
  moveStepDown: 'plan.editor.moveStepDown',
  noSteps: 'plan.editor.noSteps',
  unchecked: 'plan.editor.unchecked',
  family: 'plan.editor.family',
  format: 'plan.editor.format',
  sport: 'plan.editor.sport',
  testedRange: 'plan.editor.testedRange',
  noFamilyForSport: 'plan.editor.noFamilyForSport',
  derivedHeading: 'plan.editor.derivedHeading',
  minutes: 'plan.editor.minutes',
  stepsHeading: 'plan.editor.stepsHeading',
  findingsHeading: 'plan.editor.findingsHeading',
  findingsInside: 'plan.editor.findingsInside',
  findingsNote: 'plan.editor.findingsNote',
  draftNote: 'plan.editor.draftNote',
  replacesWith: 'plan.editor.replacesWith',
  replacesWithRest: 'plan.editor.replacesWithRest',
  replacesRest: 'plan.editor.replacesRest',
  noChanges: 'plan.editor.noChanges',
  discardTitle: 'plan.editor.discardTitle',
  discardBody: 'plan.editor.discardBody',
  discardConfirm: 'plan.editor.discardConfirm',
  keepEditing: 'plan.editor.keepEditing',
  edited: 'plan.editor.edited',
  editedNote: 'plan.editor.editedNote',
  addedNote: 'plan.editor.addedNote',
  restore: 'plan.editor.restore',
  restoreNote: 'plan.editor.restoreNote',
  restoreNoteRest: 'plan.editor.restoreNoteRest',
  restored: 'plan.editor.restored',
  restoreFailed: 'plan.editor.restoreFailed',
  measuring: 'plan.editor.measuring',
  previewFailed: 'plan.editor.previewFailed',
  save: 'plan.editor.save',
  saving: 'plan.editor.saving',
  saveFailed: 'plan.editor.saveFailed',
  loading: 'plan.editor.loading',
  noFamilies: 'plan.editor.noFamilies',
  loadFailed: 'plan.editor.loadFailed',
  cancel: 'common.cancel',
  restDayLabel: 'chat.restDay',
  sport_ride: 'app.sportRide',
  sport_run: 'app.sportRun',
  sport_swim: 'app.sportSwim',
} as const;

/** The label key of each stated value. */
export const SESSION_PARAM_LABEL_KEY: Record<SessionParamKey, string> = {
  sets: 'plan.editor.param.sets',
  reps: 'plan.editor.param.reps',
  work_seconds: 'plan.editor.param.workSeconds',
  rest_seconds: 'plan.editor.param.restSeconds',
  set_rest_seconds: 'plan.editor.param.setRestSeconds',
  duration_minutes: 'plan.editor.param.durationMinutes',
};

/** The label key of a sport. */
export function sessionSportLabelKey(sport: SessionSport): string {
  return SESSION_EDITOR_KEYS[`sport_${sport}`];
}

/** The family a build names, when the catalogue offers it. */
export function familyOf(catalogue: FamilyCatalogueResponse, build: SessionBuild): WorkoutFamily | undefined {
  return catalogue.families.find((family) => family.slug === build.family);
}

/** The preset a build resolves to: the one it names, else its family's default. */
export function presetOf(catalogue: FamilyCatalogueResponse, build: SessionBuild): FamilyPreset | undefined {
  const family = familyOf(catalogue, build);
  if (!family) return undefined;
  const key = build.preset ?? family.default_preset;
  return family.presets.find((preset) => preset.key === key);
}

/**
 * The values a preset lets the coach state in `sport`, in the editor's order,
 * with their evidence ranges in that sport.
 */
export function editableParams(
  preset: FamilyPreset | undefined,
  sport: SessionSport,
): { key: SessionParamKey; range: SessionParamRange }[] {
  const ranges = preset?.params[sport];
  if (!ranges) return [];
  return SESSION_PARAM_KEYS.flatMap((key) => {
    const range = ranges[key];
    return range ? [{ key, range }] : [];
  });
}

/**
 * The stated values the kernel found outside their preset's evidence range,
 * read from the preview's findings: the fields the editor marks. Saved all
 * the same (D3); the client never compares a value with a range itself.
 */
export function flaggedParams(preview: SessionPreview | undefined): ReadonlySet<SessionParamKey> {
  const flagged = new Set<SessionParamKey>();
  for (const finding of preview?.findings ?? []) {
    if (finding.param !== undefined) flagged.add(finding.param);
  }
  return flagged;
}

/** The value a key reads in the editor: what the coach stated, else the preset's default. */
export function paramValue(build: SessionBuild, key: SessionParamKey, range: SessionParamRange): number {
  return build.params?.[key] ?? range.default;
}

/** A preset's defaults in `sport`, as stated values, so the preview measures what the inputs show. */
function defaultsOf(preset: FamilyPreset | undefined, sport: SessionSport): SessionBuild['params'] {
  const params: Partial<Record<SessionParamKey, number>> = {};
  for (const { key, range } of editableParams(preset, sport)) params[key] = range.default;
  return params;
}

/** The sport a preset is built in: the one asked for when it has that form, else its first. */
function sportFor(preset: FamilyPreset | undefined, wanted: SessionSport): SessionSport {
  if (!preset || preset.sports.length === 0 || preset.sports.includes(wanted)) return wanted;
  return preset.sports[0] ?? wanted;
}

/**
 * The build the editor opens on for `day`: the day's own family, preset,
 * values and sport when it instantiates a family the catalogue offers, else
 * the first family in the day's sport (or the first at all) at its defaults.
 * `null` when the catalogue offers no family.
 */
export function initialSessionBuild(day: PlanDay | null, catalogue: FamilyCatalogueResponse): SessionBuild | null {
  const daySport = day?.family_sport;
  const own = day?.template_slug ? catalogue.families.find((f) => f.slug === day.template_slug) : undefined;
  if (own) {
    const preset = own.presets.find((p) => p.key === day?.template_preset) ?? own.presets.find((p) => p.key === own.default_preset);
    const sport = sportFor(preset, daySport ?? preset?.sports[0] ?? 'ride');
    return {
      family: own.slug,
      preset: preset?.key ?? own.default_preset,
      sport,
      params: { ...defaultsOf(preset, sport), ...(day?.template_params ?? {}) },
    };
  }
  const family =
    (daySport ? catalogue.families.find((f) => f.sports.includes(daySport)) : undefined) ?? catalogue.families[0];
  if (!family) return null;
  return selectFamily(family, daySport ?? family.sports[0] ?? 'ride');
}

/** A new family picked: its default preset, in the current sport when it has that form, at that sport's defaults. */
export function selectFamily(family: WorkoutFamily, sport: SessionSport): SessionBuild {
  const preset = family.presets.find((p) => p.key === family.default_preset) ?? family.presets[0];
  const built = sportFor(preset, sport);
  return {
    family: family.slug,
    preset: preset?.key ?? family.default_preset,
    sport: built,
    params: defaultsOf(preset, built),
  };
}

/** A new preset picked: in the current sport when it has that form, at that sport's defaults. */
export function selectPreset(build: SessionBuild, preset: FamilyPreset): SessionBuild {
  const sport = sportFor(preset, build.sport);
  return { ...build, preset: preset.key, sport, params: defaultsOf(preset, sport) };
}

/**
 * A sport picked: the values go back to the preset's defaults in that sport,
 * since its ranges there are its own (a 30/15 runs fewer sets than it rides).
 */
export function selectSport(build: SessionBuild, preset: FamilyPreset | undefined, sport: SessionSport): SessionBuild {
  return { ...build, sport, params: defaultsOf(preset, sport) };
}

/**
 * One value stated. Any whole number goes through, inside the preset's range
 * or not (D3: the preview's findings say what it costs). `undefined` clears
 * it back to the preset's default.
 */
export function setParam(build: SessionBuild, key: SessionParamKey, value: number | undefined): SessionBuild {
  const params = { ...(build.params ?? {}) };
  if (value === undefined || !Number.isFinite(value)) {
    delete params[key];
  } else {
    params[key] = Math.max(0, Math.round(value));
  }
  return { ...build, params };
}

/** The sports a family in the catalogue has a form for, in the editor's order. */
export function catalogueSports(catalogue: FamilyCatalogueResponse): SessionSport[] {
  return SESSION_SPORTS.filter((sport) => catalogue.families.some((family) => family.sports.includes(sport)));
}

/** The families with a form in `sport`; every family when the day reads as none of the three. */
export function familiesForSport(catalogue: FamilyCatalogueResponse, sport: SessionSport | undefined): WorkoutFamily[] {
  if (sport === undefined) return catalogue.families;
  return catalogue.families.filter((family) => family.sports.includes(sport));
}

/** A family's formats with a form in `sport`. */
export function presetsForSport(family: WorkoutFamily | undefined, sport: SessionSport): FamilyPreset[] {
  return (family?.presets ?? []).filter((preset) => preset.sports.length === 0 || preset.sports.includes(sport));
}

/**
 * A sport picked first: the build stays on its family and format when they
 * have that form, else moves to the family's first format that has it, else
 * to the first family that has it — each at that sport's defaults. The build
 * is unchanged when no family has the sport.
 */
export function selectBuildSport(
  catalogue: FamilyCatalogueResponse,
  build: SessionBuild,
  sport: SessionSport,
): SessionBuild {
  const family = familyOf(catalogue, build);
  if (family?.sports.includes(sport)) {
    const preset = presetOf(catalogue, build);
    if (preset && (preset.sports.length === 0 || preset.sports.includes(sport))) {
      return selectSport(build, preset, sport);
    }
    const first = presetsForSport(family, sport)[0];
    if (first) return selectPreset({ ...build, sport }, first);
  }
  const other = familiesForSport(catalogue, sport)[0];
  return other ? selectFamily(other, sport) : build;
}

/** Whether a family build differs from what the day already holds: always, on a day no family built. */
export function familyBuildChanged(day: PlanDay, initial: SessionBuild, build: SessionBuild): boolean {
  if (!day.template_slug) return true;
  return JSON.stringify(normalBuild(initial)) !== JSON.stringify(normalBuild(build));
}

function normalBuild(build: SessionBuild): SessionBuild {
  const params = Object.fromEntries(
    Object.entries(build.params ?? {}).sort(([a], [b]) => a.localeCompare(b)),
  ) as SessionBuild['params'];
  return { family: build.family, preset: build.preset, sport: build.sport, params };
}

/**
 * Whether closing the sheet would lose an edit: the day's own session changed,
 * or a family build the coach touched that differs from what the day holds.
 * A family build the coach never touched loses nothing.
 */
export function editorDirty(
  day: PlanDay,
  hand: HandSession,
  opening: SessionBuild | null,
  edited: SessionBuild | null,
): boolean {
  if (handSessionChanged(day, hand)) return true;
  return edited !== null && opening !== null && familyBuildChanged(day, opening, edited);
}

/**
 * The day's sport when no family has a form in it — the family mode then
 * says so above the build it opens on in another sport. `null` when a family
 * has the day's sport, or the day reads as none of the three.
 */
export function familyMissingForDaySport(day: PlanDay, catalogue: FamilyCatalogueResponse): SessionSport | null {
  const sport = day.family_sport;
  if (sport === undefined) return null;
  return familiesForSport(catalogue, sport).length === 0 ? sport : null;
}

/** Which way the coach edits the day: its own session by hand, or a build from a family. */
export type EditorMode = 'session' | 'family';

/** The mode a day opens in: a day a family built opens on its family, any other on its own session. */
export function initialEditorMode(day: PlanDay): EditorMode {
  return day.template_slug ? 'family' : 'session';
}

/** The day's session as the coach edits it by hand (carnet#1714). */
export interface HandSession {
  rest: boolean;
  /** The day's sport, in the plan's own words. */
  sport: string;
  workout: string;
  duration_min?: number;
  intensity: string;
  steps: PlanStep[];
}

/** The sport a rest day turned into a session starts on, until the coach picks one. */
const FIRST_SPORT: SessionSport = SESSION_SPORTS[0];

/** The day's own session, ready to edit. */
export function handSessionOf(day: PlanDay): HandSession {
  return {
    rest: day.rest,
    sport: day.rest ? FIRST_SPORT : day.sport,
    workout: day.rest ? '' : day.workout,
    duration_min: day.rest ? undefined : day.duration_min,
    intensity: day.rest ? '' : day.intensity,
    steps: day.rest ? [] : (day.steps ?? []).map((step) => ({ ...step })),
  };
}

/** The sports the hand editor offers: the day's own word first, then the three a family is written for. */
export function handSportOptions(session: HandSession): string[] {
  const options = [session.sport, ...SESSION_SPORTS];
  return options.filter(
    (option, index) =>
      option.trim().length > 0 && options.findIndex((other) => other.toLowerCase() === option.toLowerCase()) === index,
  );
}

/** A step a coach adds: five minutes, unnamed, until they write it. */
const NEW_STEP: PlanStep = { label: '', duration_seconds: 300, target_zone: '' };

/** The session with `patch` applied. */
export function setHand(session: HandSession, patch: Partial<HandSession>): HandSession {
  return { ...session, ...patch };
}

/** A step added at the end. */
export function addStep(session: HandSession): HandSession {
  return { ...session, steps: [...session.steps, { ...NEW_STEP }] };
}

/** The step at `index` removed. */
export function removeStep(session: HandSession, index: number): HandSession {
  return { ...session, steps: session.steps.filter((_, at) => at !== index) };
}

/** The step at `index` moved by `delta` places, held inside the list. */
export function moveStep(session: HandSession, index: number, delta: -1 | 1): HandSession {
  const to = index + delta;
  if (index < 0 || index >= session.steps.length || to < 0 || to >= session.steps.length) return session;
  const steps = [...session.steps];
  const [moved] = steps.splice(index, 1);
  steps.splice(to, 0, moved as PlanStep);
  return { ...session, steps };
}

/** The step at `index` with `patch` applied; `repeat: undefined` drops the repeat. */
export function updateStep(session: HandSession, index: number, patch: Partial<PlanStep>): HandSession {
  const steps = session.steps.map((step, at) => {
    if (at !== index) return step;
    const next: PlanStep = { ...step, ...patch };
    if (next.repeat === undefined) delete next.repeat;
    return next;
  });
  return { ...session, steps };
}

/** A step's duration as the field shows it: `m:ss` (`90` → `1:30`, `600` → `10:00`). */
export function formatStepDuration(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  return `${minutes}:${String(rest).padStart(2, '0')}`;
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

/**
 * Whether the hand session is whole enough to save: a session needs its
 * sport and title, and each step a name, a target, a duration and a repeat of
 * at least one. Whether the target is one the calendar resolves is the
 * server's to say; its refusal names the words it takes.
 */
export function handSessionComplete(session: HandSession): boolean {
  if (session.rest) return true;
  return (
    session.sport.trim().length > 0 &&
    session.workout.trim().length > 0 &&
    session.steps.every(
      (step) =>
        step.label.trim().length > 0 &&
        step.target_zone.trim().length > 0 &&
        step.duration_seconds > 0 &&
        (step.repeat === undefined || step.repeat >= 1),
    )
  );
}

/**
 * Whether the coach states the session's duration: only while it has no
 * steps. With steps the server writes the duration they total, and refuses
 * a stated one that differs, so the editor hides the field and sends none.
 */
export function handStatesDuration(session: HandSession): boolean {
  return !session.rest && session.steps.length === 0;
}

/** What the hand save sends for the session. */
export function handDayRequest(session: HandSession): HandDayRequest {
  if (session.rest) return { rest: true };
  return {
    rest: false,
    sport: session.sport.trim(),
    workout: session.workout.trim(),
    ...(handStatesDuration(session) && session.duration_min !== undefined ? { duration_min: session.duration_min } : {}),
    intensity: session.intensity.trim(),
    steps: session.steps.map((step) => ({ ...step, label: step.label.trim(), target_zone: step.target_zone.trim() })),
  };
}

/** Whether the coach changed the day's own session. */
export function handSessionChanged(day: PlanDay, session: HandSession): boolean {
  return JSON.stringify(handDayRequest(handSessionOf(day))) !== JSON.stringify(handDayRequest(session));
}

/** What a save would put on the day: a rest day, or a session by its name. */
export type DayReplacement = { rest: true } | { rest: false; name: string };

/**
 * The footer's sentence: the session the save replaces and what replaces it,
 * as a key and its values. `null` (nothing changed) reads "No changes yet.";
 * a session with no title yet has no sentence until it has one.
 */
export function replacementLine(
  day: PlanDay,
  next: DayReplacement | null,
): { key: string; values?: Record<string, string> } | null {
  if (next === null || (next.rest && day.rest)) return { key: SESSION_EDITOR_KEYS.noChanges };
  if (!next.rest && next.name.trim().length === 0) return null;
  if (next.rest) return { key: SESSION_EDITOR_KEYS.replacesWithRest, values: { from: day.workout } };
  if (day.rest) return { key: SESSION_EDITOR_KEYS.replacesRest, values: { to: next.name } };
  return { key: SESSION_EDITOR_KEYS.replacesWith, values: { from: day.workout, to: next.name } };
}

/**
 * The line under "Undo your edits": what restoring puts back, or that it
 * removes a session the coach added. `null` on a session the coach has not
 * edited or added.
 */
export function restoreLine(day: PlanDay): { key: string; values?: Record<string, string> } | null {
  if (day.added) return { key: SESSION_EDITOR_KEYS.restoreNoteAdded };
  if (!day.edited) return null;
  if (day.edited.rest) return { key: SESSION_EDITOR_KEYS.restoreNoteRest };
  return { key: SESSION_EDITOR_KEYS.restoreNote, values: { from: day.edited.workout } };
}

/**
 * Whether the card says a day is not checked against a family's evidence:
 * a session the coach edited that names no family. An agent-written day is
 * not checked either, and is not edited, so it carries no line.
 */
export function dayIsUncheckedEdit(day: PlanDay): boolean {
  return (day.edited !== undefined || day.added === true) && !day.rest && !day.template_slug;
}

/**
 * The most entries a date holds, a rest entry included. The server's
 * `MAX_SESSIONS_PER_DATE` is the bound and refuses past it (`date_full`);
 * this copy only hides "Add a session" on a full date (carnet#1716).
 */
export const MAX_SESSIONS_PER_DATE = 5;

/** The refusal reason of a write whose position now holds another session (carnet#1716). */
export const SESSION_MOVED_REASON = 'session_moved';

/** Whether a refused write found its session moved: the sheet then says so and the card reads the plan again. */
export function isSessionMoved(error: unknown): boolean {
  return refusalReason(error) === SESSION_MOVED_REASON;
}

/** The refusal reason of an add on a date that already holds `MAX_SESSIONS_PER_DATE` entries (carnet#1716). */
export const DATE_FULL_REASON = 'date_full';

/** Whether a refused add found its date full: the sheet then says `dateFull` rather than the server's sentence. */
export function isDateFull(error: unknown): boolean {
  return refusalReason(error) === DATE_FULL_REASON;
}

/** Where a card entry sits among its date's entries; a card written before positions reads as 0. */
export function dayPosition(day: PlanDay): number {
  return day.position ?? 0;
}

/** The session a write is about, as the card shows it. */
export function sessionExpect(day: PlanDay): SessionExpect {
  return { sport: day.sport, workout: day.workout };
}

/** Whether a date's entries leave room for another session. */
export function canAddSession(entries: readonly PlanDay[]): boolean {
  return entries.length < MAX_SESSIONS_PER_DATE;
}

/** Whether one of a date's sessions may be removed: never its last entry, which the Rest switch changes instead. */
export function canRemoveSession(entries: readonly PlanDay[]): boolean {
  return entries.length > 1;
}

/**
 * The blank session "Add a session" opens on, as a day the editor starts
 * from: in the sport of the date's first session, else the first a family is
 * written for, with no title yet.
 */
export function newSessionDay(date: string, entries: readonly PlanDay[]): PlanDay {
  const sport = entries.find((entry) => !entry.rest)?.sport ?? FIRST_SPORT;
  return { date, sport, workout: '', intensity: '', rest: false, position: entries.length };
}

/** One row of the derived values: its label key, the label's values, and the figure, formatted. */
export interface DerivedRow {
  key: string;
  labelKey: string;
  labelValues?: Record<string, string>;
  value: string;
}

const DERIVED_KEYS = {
  ratio: 'plan.editor.derived.ratio',
  workPct: 'plan.editor.derived.workPct',
  restPct: 'plan.editor.derived.restPct',
  cycleMeanPct: 'plan.editor.derived.cycleMeanPct',
  amplitudePct: 'plan.editor.derived.amplitudePct',
  onTime: 'plan.editor.derived.onTime',
  total: 'plan.editor.derived.total',
} as const;

/** A figure the kernel derived, as shown: whole percentages, up to two decimals for a ratio, one for minutes. */
function figure(value: number, digits: number, language: string): string {
  return new Intl.NumberFormat(language, { maximumFractionDigits: digits, minimumFractionDigits: 0 }).format(value);
}

/**
 * The rows the editor lists under "What this session does", in a fixed
 * order, leaving out what the kernel derived nothing for. Formatting only:
 * every number is the server's.
 */
export function derivedRows(
  derived: SessionDerived,
  language: string,
  minutes: (count: string) => string,
): DerivedRow[] {
  const max = { max: derived.aerobic_max };
  const rows: DerivedRow[] = [];
  if (derived.ratio !== undefined) {
    rows.push({ key: 'ratio', labelKey: DERIVED_KEYS.ratio, value: `${figure(derived.ratio, 2, language)}:1` });
  }
  const pct = (key: string, labelKey: string, value: number | undefined) => {
    if (value !== undefined) rows.push({ key, labelKey, labelValues: max, value: `${figure(value, 0, language)} %` });
  };
  pct('work_pct', DERIVED_KEYS.workPct, derived.work_pct);
  pct('rest_pct', DERIVED_KEYS.restPct, derived.rest_pct);
  pct('cycle_mean_pct', DERIVED_KEYS.cycleMeanPct, derived.cycle_mean_pct);
  if (derived.amplitude_pct !== undefined) {
    rows.push({
      key: 'amplitude_pct',
      labelKey: DERIVED_KEYS.amplitudePct,
      labelValues: max,
      value: figure(derived.amplitude_pct, 0, language),
    });
  }
  rows.push({ key: 'on_time', labelKey: DERIVED_KEYS.onTime, value: minutes(figure(derived.on_time_minutes, 1, language)) });
  rows.push({ key: 'total', labelKey: DERIVED_KEYS.total, value: minutes(figure(derived.total_minutes, 1, language)) });
  return rows;
}

/** Which session of which date a write is about, and that session as the card showed it (carnet#1716). */
export interface SessionAt {
  date: string;
  position: number;
  expect: SessionExpect;
}

/** The chat API calls the editor makes, bound by each client. */
export interface SessionEditorApis {
  chatApi: Pick<
    ChatApi,
    | 'getSubjectFamilies'
    | 'previewSubjectSession'
    | 'saveSubjectDraftDay'
    | 'saveSubjectDraftHandDay'
    | 'restoreSubjectDraftDay'
    | 'removeSubjectDraftDay'
    | 'addSubjectDraftSession'
  >;
}

/** How long the coach's typing settles before the preview measures it again. */
export const SESSION_PREVIEW_DEBOUNCE_MS = 300;

/** How long the thread's family catalogue stays fresh: it changes only with the agent's package or a deploy. */
const FAMILIES_STALE_MS = 5 * 60_000;

/** `value`, once it has stopped changing for `delayMs`. */
function useSettled<T>(value: T, delayMs: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return settled;
}

/**
 * Build the editor's hooks over one client's chat API.
 *
 * - `useSubjectFamilies` reads the thread's athlete's families in the coach's language.
 * - `useSessionPreview` measures the build once the coach's typing settles; the
 *   previous preview stays on screen while the next one is measured.
 * - `useSaveDraftDay` writes the build as that day of the coach's draft and puts
 *   the answered subject plan straight into the card's cache.
 * - `useSaveDraftHandDay` writes the session as the coach wrote it by hand, or a
 *   rest day; `useRestoreDraftDay` puts a session back as it stood before the
 *   coach's first edit, or removes one the coach added; `useRemoveDraftSession`
 *   and `useAddDraftSession` take a session off a date or add one to it. Each
 *   answers the subject plan into the same cache. Every write but the add
 *   names its session by position and as the card showed it (carnet#1716).
 * - `useRereadPlan` reads the card's plan again after a write found its
 *   session moved.
 */
export function createSessionEditorHooks({ chatApi }: SessionEditorApis) {
  function useSubjectFamilies(conversationId: string | null, locale: string) {
    return useQuery({
      queryKey: QUERY_KEYS.chat.subjectFamilies(conversationId, locale),
      queryFn: () => chatApi.getSubjectFamilies(conversationId as string, locale),
      enabled: conversationId !== null,
      staleTime: FAMILIES_STALE_MS,
    });
  }

  function useSessionPreview(conversationId: string | null, date: string, build: SessionBuild | null, locale: string) {
    const settled = useSettled(build, SESSION_PREVIEW_DEBOUNCE_MS);
    return useQuery<SessionPreview>({
      queryKey: QUERY_KEYS.chat.sessionPreview(conversationId, date, settled, locale),
      queryFn: () => chatApi.previewSubjectSession(conversationId as string, { ...(settled as SessionBuild), date }, locale),
      enabled: conversationId !== null && settled !== null,
      placeholderData: (previous) => previous,
      retry: false,
    });
  }

  // The card's read caches a `SubjectPlanRead`; each write answers the plan
  // the read would, so the card shows the written day at once.
  function useAnsweredPlan(conversationId: string | null) {
    const queryClient = useQueryClient();
    return (plan: SubjectPlanResponse) => {
      queryClient.setQueryData(QUERY_KEYS.chat.subjectPlan(conversationId), { status: 'ok', plan });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.subjectPlan(conversationId) });
    };
  }

  function useSaveDraftDay(conversationId: string | null, locale: string) {
    const onSuccess = useAnsweredPlan(conversationId);
    return useMutation<SubjectPlanResponse, unknown, SessionAt & { build: SessionBuild }>({
      mutationFn: ({ date, position, expect, build }) =>
        chatApi.saveSubjectDraftDay(conversationId as string, date, position, build, expect, locale),
      onSuccess,
    });
  }

  function useSaveDraftHandDay(conversationId: string | null, locale: string) {
    const onSuccess = useAnsweredPlan(conversationId);
    return useMutation<SubjectPlanResponse, unknown, SessionAt & { day: HandDayRequest }>({
      mutationFn: ({ date, position, expect, day }) =>
        chatApi.saveSubjectDraftHandDay(conversationId as string, date, position, day, expect, locale),
      onSuccess,
    });
  }

  function useRestoreDraftDay(conversationId: string | null, locale: string) {
    const onSuccess = useAnsweredPlan(conversationId);
    return useMutation<SubjectPlanResponse, unknown, SessionAt>({
      mutationFn: ({ date, position, expect }) =>
        chatApi.restoreSubjectDraftDay(conversationId as string, date, position, expect, locale),
      onSuccess,
    });
  }

  function useRemoveDraftSession(conversationId: string | null, locale: string) {
    const onSuccess = useAnsweredPlan(conversationId);
    return useMutation<SubjectPlanResponse, unknown, SessionAt>({
      mutationFn: ({ date, position, expect }) =>
        chatApi.removeSubjectDraftDay(conversationId as string, date, position, expect, locale),
      onSuccess,
    });
  }

  function useAddDraftSession(conversationId: string | null, locale: string) {
    const onSuccess = useAnsweredPlan(conversationId);
    return useMutation<SubjectPlanResponse, unknown, { date: string; session: AddedSession }>({
      mutationFn: ({ date, session }) => chatApi.addSubjectDraftSession(conversationId as string, date, session, locale),
      onSuccess,
    });
  }

  /**
   * After a refused write found its session moved, the card's plan is read
   * again so the coach edits what is there now.
   */
  function useRereadPlan(conversationId: string | null) {
    const queryClient = useQueryClient();
    return () => void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.subjectPlan(conversationId) });
  }

  return {
    useSubjectFamilies,
    useSessionPreview,
    useSaveDraftDay,
    useSaveDraftHandDay,
    useRestoreDraftDay,
    useRemoveDraftSession,
    useAddDraftSession,
    useRereadPlan,
  };
}
