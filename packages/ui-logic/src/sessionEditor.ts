// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's "build from family" editor on both clients — the build's state, its live preview, and the save into the draft
// ABOUTME: The kernel derives every number on the server; this only holds what the coach picked and lays out what came back (carnet#1515)

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { ChatApi } from '@pierre/api-client';
import {
  SESSION_PARAM_KEYS,
  type FamilyCatalogueResponse,
  type FamilyPreset,
  type PlanDay,
  type SessionBuild,
  type SessionDerived,
  type SessionParamKey,
  type SessionParamRange,
  type SessionPreview,
  type SessionSport,
  type SubjectPlanResponse,
  type WorkoutFamily,
} from '@pierre/shared-types';

/** The editor's i18n keys (contremaitre `strings/`, `plan.editor`). */
export const SESSION_EDITOR_KEYS = {
  open: 'plan.editor.open',
  title: 'plan.editor.title',
  family: 'plan.editor.family',
  preset: 'plan.editor.preset',
  sport: 'plan.editor.sport',
  range: 'plan.editor.range',
  derivedHeading: 'plan.editor.derivedHeading',
  minutes: 'plan.editor.minutes',
  stepsHeading: 'plan.editor.stepsHeading',
  findingsHeading: 'plan.editor.findingsHeading',
  findingsNone: 'plan.editor.findingsNone',
  findingsNote: 'plan.editor.findingsNote',
  draftNote: 'plan.editor.draftNote',
  replaces: 'plan.editor.replaces',
  measuring: 'plan.editor.measuring',
  previewFailed: 'plan.editor.previewFailed',
  save: 'plan.editor.save',
  saving: 'plan.editor.saving',
  saveFailed: 'plan.editor.saveFailed',
  loading: 'plan.editor.loading',
  noFamilies: 'plan.editor.noFamilies',
  loadFailed: 'plan.editor.loadFailed',
  cancel: 'common.cancel',
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

/** The chat API calls the editor makes, bound by each client. */
export interface SessionEditorApis {
  chatApi: Pick<ChatApi, 'getSubjectFamilies' | 'previewSubjectSession' | 'saveSubjectDraftDay'>;
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

  function useSaveDraftDay(conversationId: string | null, locale: string) {
    const queryClient = useQueryClient();
    return useMutation<SubjectPlanResponse, unknown, { date: string; build: SessionBuild }>({
      mutationFn: ({ date, build }) => chatApi.saveSubjectDraftDay(conversationId as string, date, build, locale),
      onSuccess: (plan) => {
        // The card's read caches a `SubjectPlanRead`; the write answers the
        // plan the read would, so the card shows the saved day at once.
        queryClient.setQueryData(QUERY_KEYS.chat.subjectPlan(conversationId), { status: 'ok', plan });
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.subjectPlan(conversationId) });
      },
    });
  }

  return { useSubjectFamilies, useSessionPreview, useSaveDraftDay };
}
