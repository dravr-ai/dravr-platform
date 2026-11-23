// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the coach's "build from family" editor logic — the build a day opens on, the picks, the values, the derived rows
// ABOUTME: Both clients render from these; a value outside its range is kept (D3) and every figure is the server's, only formatted here

import { describe, it, expect, vi, afterEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type {
  FamilyCatalogueResponse,
  PlanDay,
  SessionBuild,
  SessionDerived,
  SessionPreview,
  SubjectPlanResponse,
} from '@pierre/shared-types';
import {
  SESSION_PREVIEW_DEBOUNCE_MS,
  createSessionEditorHooks,
  derivedRows,
  editableParams,
  initialSessionBuild,
  flaggedParams,
  paramValue,
  presetOf,
  selectFamily,
  selectPreset,
  selectSport,
  sessionSportLabelKey,
  setParam,
  type SessionEditorApis,
} from '../src/sessionEditor';

/** Two families shaped like the catalogue the server answers: a short-interval family and an endurance one. */
const CATALOGUE: FamilyCatalogueResponse = {
  families: [
    {
      slug: 'vo2max-short',
      name: 'Short VO2max intervals',
      purpose: 'vo2max_short',
      purpose_label: 'VO2max, short intervals',
      sports: ['ride', 'run'],
      default_preset: 'ronnestad-30-15',
      presets: [
        {
          key: 'ronnestad-30-15',
          name: '30/15 (Rønnestad)',
          sports: ['ride', 'run'],
          params: {
            ride: {
              sets: { min: 2, max: 4, default: 3 },
              reps: { min: 9, max: 15, default: 13 },
              work_seconds: { min: 30, max: 30, default: 30 },
              rest_seconds: { min: 15, max: 15, default: 15 },
              set_rest_seconds: { min: 120, max: 300, default: 180 },
            },
            run: {
              sets: { min: 2, max: 3, default: 2 },
              reps: { min: 9, max: 15, default: 13 },
              work_seconds: { min: 30, max: 30, default: 30 },
              rest_seconds: { min: 15, max: 15, default: 15 },
              set_rest_seconds: { min: 120, max: 300, default: 180 },
            },
          },
        },
        {
          key: 'billat-30-30',
          name: '30/30 (Billat)',
          sports: ['run'],
          params: {
            run: {
              reps: { min: 10, max: 20, default: 15 },
              work_seconds: { min: 30, max: 30, default: 30 },
              rest_seconds: { min: 30, max: 30, default: 30 },
            },
          },
        },
      ],
    },
    {
      slug: 'endurance',
      name: 'Endurance',
      purpose: 'endurance',
      sports: ['ride', 'run', 'swim'],
      default_preset: 'steady',
      presets: [
        {
          key: 'steady',
          name: 'Steady',
          sports: ['ride', 'run', 'swim'],
          params: { ride: { duration_minutes: { min: 30, max: 240, default: 90 } }, run: { duration_minutes: { min: 30, max: 240, default: 90 } }, swim: { duration_minutes: { min: 30, max: 240, default: 90 } } },
        },
      ],
    },
  ],
};

function day(overrides: Partial<PlanDay> = {}): PlanDay {
  return {
    date: '2026-11-24',
    sport: 'run',
    workout: '30/15 intervals',
    duration_min: 50,
    intensity: 'hard',
    rest: false,
    ...overrides,
  };
}

describe('initialSessionBuild', () => {
  it("opens on the day's own family, preset, values and sport, the preset's defaults filling the rest", () => {
    const build = initialSessionBuild(
      day({
        template_slug: 'vo2max-short',
        template_preset: 'ronnestad-30-15',
        template_params: { reps: 11, sets: 2 },
        family_sport: 'run',
      }),
      CATALOGUE,
    );

    expect(build).toEqual({
      family: 'vo2max-short',
      preset: 'ronnestad-30-15',
      sport: 'run',
      params: { sets: 2, reps: 11, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
    });
  });

  it("takes the family's default preset when the day names none", () => {
    const build = initialSessionBuild(day({ template_slug: 'vo2max-short', family_sport: 'ride' }), CATALOGUE);

    expect(build?.preset).toBe('ronnestad-30-15');
    expect(build?.sport).toBe('ride');
    expect(build?.params).toEqual({ sets: 3, reps: 13, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 });
  });

  it('opens a rest day on the first family at its defaults, in that family’s first sport', () => {
    const build = initialSessionBuild(
      day({ rest: true, sport: 'rest', workout: 'Rest', duration_min: undefined, intensity: '' }),
      CATALOGUE,
    );

    expect(build).toEqual({
      family: 'vo2max-short',
      preset: 'ronnestad-30-15',
      sport: 'ride',
      params: { sets: 3, reps: 13, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
    });
  });

  it("opens a day with no family on the first family offering the day's sport", () => {
    const build = initialSessionBuild(day({ sport: 'swim', workout: 'Easy swim', family_sport: 'swim' }), CATALOGUE);

    expect(build).toEqual({ family: 'endurance', preset: 'steady', sport: 'swim', params: { duration_minutes: 90 } });
  });

  it('treats a family the catalogue no longer offers like a day with none', () => {
    const build = initialSessionBuild(
      day({ template_slug: 'retired-family', template_preset: 'x', family_sport: 'swim' }),
      CATALOGUE,
    );

    expect(build?.family).toBe('endurance');
    expect(build?.sport).toBe('swim');
  });

  it('has nothing to open on when the catalogue offers no family', () => {
    expect(initialSessionBuild(day(), { families: [] })).toBeNull();
  });
});

describe('the picks', () => {
  const vo2 = CATALOGUE.families[0];
  const billat = vo2.presets[1];

  it("picks a family's default preset at its defaults, keeping the sport when the preset has that form", () => {
    expect(selectFamily(CATALOGUE.families[1], 'swim')).toEqual({
      family: 'endurance',
      preset: 'steady',
      sport: 'swim',
      params: { duration_minutes: 90 },
    });
    // The defaults are the sport's own: the 30/15 starts on two sets running, three riding.
    expect(selectFamily(CATALOGUE.families[0], 'run').params?.sets).toBe(2);
    expect(selectFamily(CATALOGUE.families[0], 'ride').params?.sets).toBe(3);
  });

  it("resets the values to the new preset's defaults and moves to a sport it has a form for", () => {
    const stated: SessionBuild = {
      family: 'vo2max-short',
      preset: 'ronnestad-30-15',
      sport: 'ride',
      params: { sets: 4, reps: 20, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 240 },
    };

    expect(selectPreset(stated, billat)).toEqual({
      family: 'vo2max-short',
      preset: 'billat-30-30',
      sport: 'run',
      params: { reps: 15, work_seconds: 30, rest_seconds: 30 },
    });
  });

  it("changes the sport and goes back to the preset's defaults in that sport", () => {
    const ronnestad = CATALOGUE.families[0].presets[0];
    const stated: SessionBuild = { family: 'vo2max-short', preset: 'ronnestad-30-15', sport: 'ride', params: { reps: 11 } };

    // The 30/15 runs fewer sets than it rides: its run form has its own ranges.
    expect(selectSport(stated, ronnestad, 'run')).toEqual({
      ...stated,
      sport: 'run',
      params: { sets: 2, reps: 13, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
    });
  });

  it("resolves a build naming no preset to its family's default", () => {
    expect(presetOf(CATALOGUE, { family: 'vo2max-short', sport: 'ride' })?.key).toBe('ronnestad-30-15');
    expect(presetOf(CATALOGUE, { family: 'unknown', sport: 'ride' })).toBeUndefined();
  });

  it('names each sport with the app’s own sport words', () => {
    expect(sessionSportLabelKey('ride')).toBe('app.sportRide');
    expect(sessionSportLabelKey('run')).toBe('app.sportRun');
    expect(sessionSportLabelKey('swim')).toBe('app.sportSwim');
  });
});

describe('the values', () => {
  const ronnestad = CATALOGUE.families[0].presets[0];
  const build: SessionBuild = {
    family: 'vo2max-short',
    preset: 'ronnestad-30-15',
    sport: 'ride',
    params: { sets: 3, reps: 13 },
  };

  it("lists the preset's knobs in the editor's order, and none for a missing preset", () => {
    expect(editableParams(ronnestad, 'ride').map((p) => p.key)).toEqual([
      'sets',
      'reps',
      'work_seconds',
      'rest_seconds',
      'set_rest_seconds',
    ]);
    expect(editableParams(undefined, 'ride')).toEqual([]);
    // A sport the preset has no form for has no knobs.
    expect(editableParams(CATALOGUE.families[0].presets[1], 'ride')).toEqual([]);
  });

  it('keeps a value outside the evidence range as stated (D3)', () => {
    const range = ronnestad.params.ride!.reps!;
    const stated = setParam(build, 'reps', 40);

    expect(stated.params?.reps).toBe(40);
    expect(paramValue(stated, 'reps', range)).toBe(40);
  });

  it('marks only the fields the kernel found outside their range', () => {
    const measured = (findings: SessionPreview['findings']): SessionPreview => ({
      family: 'vo2max-short',
      preset: 'ronnestad-30-15',
      sport: 'ride',
      duration_min: 45,
      steps: [],
      findings,
    });
    expect(flaggedParams(undefined).size).toBe(0);
    expect(flaggedParams(measured([])).size).toBe(0);
    const flagged = flaggedParams(
      measured([
        { code: 'param_out_of_range', text: '40 reps is past the tested range.', param: 'reps' },
        // A finding about the session as a whole names no field.
        { code: 'cycle_mean_low', text: 'The cycle mean sits low.' },
      ]),
    );
    expect([...flagged]).toEqual(['reps']);
  });

  it('states whole numbers no lower than zero, and nothing more', () => {
    expect(setParam(build, 'reps', 12.6).params?.reps).toBe(13);
    expect(setParam(build, 'reps', -3).params?.reps).toBe(0);
  });

  it("clears a value back to the preset's default", () => {
    const range = ronnestad.params.ride!.reps!;
    const cleared = setParam(build, 'reps', undefined);

    expect(cleared.params).toEqual({ sets: 3 });
    expect(paramValue(cleared, 'reps', range)).toBe(13);
  });

  it('leaves the build it was handed untouched', () => {
    setParam(build, 'sets', 4);

    expect(build.params).toEqual({ sets: 3, reps: 13 });
  });
});

describe('derivedRows', () => {
  const minutes = (count: string) => `${count} min`;
  const FULL: SessionDerived = {
    sets: 3,
    reps: 13,
    work_seconds: 30,
    rest_seconds: 15,
    ratio: 2,
    work_pct: 118.4,
    rest_pct: 52.6,
    cycle_mean_pct: 96.5,
    amplitude_pct: 65.8,
    on_time_minutes: 19.5,
    total_minutes: 42,
    aerobic_max: 'MAP',
  };

  it('lists every derived figure in a fixed order, formatted, the percentages naming the aerobic maximum', () => {
    expect(derivedRows(FULL, 'en', minutes)).toEqual([
      { key: 'ratio', labelKey: 'plan.editor.derived.ratio', value: '2:1' },
      { key: 'work_pct', labelKey: 'plan.editor.derived.workPct', labelValues: { max: 'MAP' }, value: '118 %' },
      { key: 'rest_pct', labelKey: 'plan.editor.derived.restPct', labelValues: { max: 'MAP' }, value: '53 %' },
      {
        key: 'cycle_mean_pct',
        labelKey: 'plan.editor.derived.cycleMeanPct',
        labelValues: { max: 'MAP' },
        value: '97 %',
      },
      { key: 'amplitude_pct', labelKey: 'plan.editor.derived.amplitudePct', labelValues: { max: 'MAP' }, value: '66' },
      { key: 'on_time', labelKey: 'plan.editor.derived.onTime', value: '19.5 min' },
      { key: 'total', labelKey: 'plan.editor.derived.total', value: '42 min' },
    ]);
  });

  it('leaves out what the kernel derived nothing for', () => {
    const endurance: SessionDerived = {
      sets: 1,
      reps: 1,
      work_seconds: 5400,
      rest_seconds: 0,
      on_time_minutes: 90,
      total_minutes: 90,
      aerobic_max: 'vVO2max',
    };

    expect(derivedRows(endurance, 'en', minutes).map((row) => row.key)).toEqual(['on_time', 'total']);
  });

  it("writes the figures in the reader's language", () => {
    const rows = derivedRows({ ...FULL, ratio: 2.67, on_time_minutes: 9.75 }, 'fr', minutes);

    expect(rows.find((row) => row.key === 'ratio')?.value).toBe('2,67:1');
    expect(rows.find((row) => row.key === 'on_time')?.value).toBe('9,8 min');
  });
});

describe('createSessionEditorHooks', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  /** The session a write is about, as the card showed it. */
  const EXPECT = { sport: 'run', workout: 'Easy run' };

  const BUILD: SessionBuild = {
    family: 'vo2max-short',
    preset: 'ronnestad-30-15',
    sport: 'ride',
    params: { sets: 3, reps: 13 },
  };

  function preview(reps: number): SessionPreview {
    return {
      family: 'vo2max-short',
      preset: 'ronnestad-30-15',
      sport: 'ride',
      duration_min: 42,
      steps: [{ label: 'Work', duration_seconds: 30, target_zone: 'Z5', repeat: reps }],
      findings: [],
    };
  }

  function harness() {
    const chatApi = {
      getSubjectFamilies: vi.fn().mockResolvedValue(CATALOGUE),
      previewSubjectSession: vi.fn((_id: string, request: { params?: { reps?: number } }) =>
        Promise.resolve(preview(request.params?.reps ?? 0)),
      ),
      saveSubjectDraftDay: vi.fn(),
      saveSubjectDraftHandDay: vi.fn(),
      restoreSubjectDraftDay: vi.fn(),
      removeSubjectDraftDay: vi.fn(),
      addSubjectDraftSession: vi.fn(),
    };
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client: queryClient }, children);
    const hooks = createSessionEditorHooks({ chatApi } as unknown as SessionEditorApis);
    return { chatApi, queryClient, wrapper, hooks };
  }

  it("reads the thread's families in the reader's language, and nothing before a thread exists", async () => {
    const { chatApi, wrapper, hooks } = harness();
    const { result, rerender } = renderHook(({ id }) => hooks.useSubjectFamilies(id, 'fr'), {
      wrapper,
      initialProps: { id: null as string | null },
    });

    expect(chatApi.getSubjectFamilies).not.toHaveBeenCalled();
    rerender({ id: 'conv-1' });
    await waitFor(() => expect(result.current.data?.families).toHaveLength(2));
    expect(chatApi.getSubjectFamilies).toHaveBeenCalledWith('conv-1', 'fr');
  });

  it('measures a change once the typing settles, keeping the previous preview on screen meanwhile', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { chatApi, wrapper, hooks } = harness();
    const { result, rerender } = renderHook(
      ({ build }) => hooks.useSessionPreview('conv-1', '2026-11-24', build, 'en'),
      { wrapper, initialProps: { build: BUILD } },
    );

    await waitFor(() => expect(result.current.data?.steps[0].repeat).toBe(13));
    expect(chatApi.previewSubjectSession).toHaveBeenCalledWith('conv-1', { ...BUILD, date: '2026-11-24' }, 'en');

    const changed = setParam(BUILD, 'reps', 20);
    rerender({ build: changed });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(SESSION_PREVIEW_DEBOUNCE_MS - 50);
    });
    expect(chatApi.previewSubjectSession).toHaveBeenCalledTimes(1);
    expect(result.current.data?.steps[0].repeat).toBe(13);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    await waitFor(() => expect(result.current.data?.steps[0].repeat).toBe(20));
    expect(chatApi.previewSubjectSession).toHaveBeenLastCalledWith(
      'conv-1',
      { ...changed, date: '2026-11-24' },
      'en',
    );
  });

  it('saves the build as that day of the draft and puts the answered plan in the card’s cache', async () => {
    const { chatApi, queryClient, wrapper, hooks } = harness();
    const answered: SubjectPlanResponse = {
      athlete: { user_id: 'user-alice', display_name: 'Alice Runner' },
      today: '2026-11-21',
      live: null,
      draft: null,
      draft_state: null,
    };
    chatApi.saveSubjectDraftDay.mockResolvedValue(answered);
    const { result } = renderHook(() => hooks.useSaveDraftDay('conv-1', 'en'), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ date: '2026-11-24', position: 1, expect: EXPECT, build: BUILD });
    });

    expect(chatApi.saveSubjectDraftDay).toHaveBeenCalledWith('conv-1', '2026-11-24', 1, BUILD, EXPECT, 'en');
    expect(queryClient.getQueryData(QUERY_KEYS.chat.subjectPlan('conv-1'))).toEqual({ status: 'ok', plan: answered });
  });

  it('saves a hand-written day and restores a day, each answering the plan into the card’s cache (carnet#1714)', async () => {
    const { chatApi, queryClient, wrapper, hooks } = harness();
    const answered: SubjectPlanResponse = {
      athlete: { user_id: 'user-alice', display_name: 'Alice Runner' },
      today: '2026-11-21',
      live: null,
      draft: null,
      draft_state: null,
    };
    const restored: SubjectPlanResponse = { ...answered, today: '2026-11-22' };
    chatApi.saveSubjectDraftHandDay.mockResolvedValue(answered);
    chatApi.restoreSubjectDraftDay.mockResolvedValue(restored);
    const hand = renderHook(() => hooks.useSaveDraftHandDay('conv-1', 'en'), { wrapper });
    const restore = renderHook(() => hooks.useRestoreDraftDay('conv-1', 'en'), { wrapper });
    const day = { rest: false as const, sport: 'run', workout: 'Easy 50 min', duration_min: 50, intensity: 'easy', steps: [] };

    await act(async () => {
      await hand.result.current.mutateAsync({ date: '2026-11-24', position: 0, expect: EXPECT, day });
    });
    expect(chatApi.saveSubjectDraftHandDay).toHaveBeenCalledWith('conv-1', '2026-11-24', 0, day, EXPECT, 'en');
    expect(queryClient.getQueryData(QUERY_KEYS.chat.subjectPlan('conv-1'))).toEqual({ status: 'ok', plan: answered });

    await act(async () => {
      await restore.result.current.mutateAsync({ date: '2026-11-24', position: 0, expect: EXPECT });
    });
    expect(chatApi.restoreSubjectDraftDay).toHaveBeenCalledWith('conv-1', '2026-11-24', 0, EXPECT, 'en');
    expect(queryClient.getQueryData(QUERY_KEYS.chat.subjectPlan('conv-1'))).toEqual({ status: 'ok', plan: restored });
  });

  it('removes a session and adds one, each answering the plan into the card’s cache (carnet#1716)', async () => {
    const { chatApi, queryClient, wrapper, hooks } = harness();
    const answered: SubjectPlanResponse = {
      athlete: { user_id: 'user-alice', display_name: 'Alice Runner' },
      today: '2026-11-21',
      live: null,
      draft: null,
      draft_state: null,
    };
    chatApi.removeSubjectDraftDay.mockResolvedValue(answered);
    chatApi.addSubjectDraftSession.mockResolvedValue({ ...answered, today: '2026-11-23' });
    const remove = renderHook(() => hooks.useRemoveDraftSession('conv-1', 'en'), { wrapper });
    const add = renderHook(() => hooks.useAddDraftSession('conv-1', 'en'), { wrapper });

    await act(async () => {
      await remove.result.current.mutateAsync({ date: '2026-11-24', position: 1, expect: EXPECT });
    });
    expect(chatApi.removeSubjectDraftDay).toHaveBeenCalledWith('conv-1', '2026-11-24', 1, EXPECT, 'en');
    expect(queryClient.getQueryData(QUERY_KEYS.chat.subjectPlan('conv-1'))).toEqual({ status: 'ok', plan: answered });

    const session = { day: { rest: false as const, sport: 'strength', workout: 'Core', intensity: '', steps: [] } };
    await act(async () => {
      await add.result.current.mutateAsync({ date: '2026-11-24', session });
    });
    expect(chatApi.addSubjectDraftSession).toHaveBeenCalledWith('conv-1', '2026-11-24', session, 'en');
    expect(queryClient.getQueryData(QUERY_KEYS.chat.subjectPlan('conv-1'))).toMatchObject({ plan: { today: '2026-11-23' } });
  });
});
