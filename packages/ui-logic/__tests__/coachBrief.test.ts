// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the morning brief's rules — the status line per outcome and state, its hours, the lists a change replaces, and the hooks (carnet#1791)
// ABOUTME: Words run on a key-echoing translator so a catalogue edit cannot hide a rule that changed; the catalogue test names every key each locale owes

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachBrief, CoachBriefUpdate } from '@pierre/shared-types';
import { COACH_BRIEF_REASON_LABEL_KEY } from '@pierre/shared-constants';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  BRIEF_HOURS,
  BRIEF_REASON_KINDS,
  applyBriefUpdate,
  briefHourLabel,
  briefStatusLine,
  briefStatusWords,
  isBriefMuted,
  isCoachBrief,
  mutedAthletes,
  withMuted,
  withReason,
} from '../src/coachBrief';
import { createCoachBriefHooks } from '../src/coachBriefHooks';

const echo = (key: string, options?: Record<string, unknown>) =>
  options === undefined ? key : `${key}${JSON.stringify(options)}`;

function brief(over: Partial<CoachBrief> = {}): CoachBrief {
  return { state: 'active', hour: 7, reasons_off: [], muted: [], last_run: null, next_run: null, ...over };
}

/** The run's local HH:mm on this machine's clock, as the line must print it. */
function localTime(instant: string): string {
  const at = new Date(instant);
  return `${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(2, '0')}`;
}

const RAN_AT = '2026-11-24T12:04:00Z';

describe('briefStatusWords', () => {
  it('says paused, with no schedule beside it, whatever the last run was', () => {
    const words = briefStatusWords(echo, 'en', brief({ state: 'paused', last_run: { ran_at: RAN_AT, outcome: 'posted', items: 3, not_sharing: 0, unreadable: 0, missed_unreadable: 0, missed_off: 0 } }));
    expect(words).toEqual({ status: 'coachHome.brief.status_paused', schedule: null });
    expect(briefStatusLine(words)).toBe('coachHome.brief.status_paused');
  });

  it('says it has not run yet while there is no last run, with its hour as HH:00', () => {
    expect(briefStatusWords(echo, 'en', brief({ hour: 6 }))).toEqual({
      status: 'coachHome.brief.status_never',
      schedule: 'coachHome.brief.schedule{"time":"06:00"}',
    });
  });

  it('names an empty run with the local time it ran', () => {
    const words = briefStatusWords(echo, 'en', brief({ last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0, not_sharing: 0, unreadable: 0, missed_unreadable: 0, missed_off: 0 } }));
    expect(words.status).toBe(`coachHome.brief.status_empty{"time":"${localTime(RAN_AT)}"}`);
  });

  it('names an empty run that left athletes out with how many, never as all fine', () => {
    const words = briefStatusWords(echo, 'en', brief({ last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0, not_sharing: 2, unreadable: 1, missed_unreadable: 0, missed_off: 0 } }));
    expect(words.status).toBe(`coachHome.brief.status_empty_left_out{"time":"${localTime(RAN_AT)}","count":3}`);
  });

  it('names a posted run with its time and how many it named', () => {
    const words = briefStatusWords(echo, 'en', brief({ last_run: { ran_at: RAN_AT, outcome: 'posted', items: 3, not_sharing: 0, unreadable: 0, missed_unreadable: 0, missed_off: 0 } }));
    expect(words.status).toBe(`coachHome.brief.status_posted{"time":"${localTime(RAN_AT)}","count":3}`);
    expect(briefStatusLine(words)).toBe(`${words.status} · coachHome.brief.schedule{"time":"07:00"}`);
  });

  it('reads held, withheld and skipped alike: held back at the time it ran', () => {
    for (const outcome of ['held', 'withheld', 'skipped'] as const) {
      const words = briefStatusWords(echo, 'en', brief({ last_run: { ran_at: RAN_AT, outcome, items: 2, not_sharing: 0, unreadable: 0, missed_unreadable: 0, missed_off: 0 } }));
      expect(words.status).toBe(`coachHome.brief.status_held{"time":"${localTime(RAN_AT)}"}`);
    }
  });

  it('closes a posted run on whom it did not check for missed sessions: an unread calendar, then a check switched off', () => {
    const words = briefStatusWords(echo, 'en', brief({ last_run: { ran_at: RAN_AT, outcome: 'posted', items: 3, not_sharing: 0, unreadable: 0, missed_unreadable: 2, missed_off: 1 } }));
    expect(words.status).toBe(
      `coachHome.brief.status_posted{"time":"${localTime(RAN_AT)}","count":3}` +
        ' · coachHome.brief.missed_unreadable{"count":2}' +
        ' · coachHome.brief.missed_off{"count":1}',
    );
    expect(briefStatusLine(words)).toBe(`${words.status} · coachHome.brief.schedule{"time":"07:00"}`);
  });

  it('names each count on its own and never adds it to the athletes an empty run left out', () => {
    const offOnly = briefStatusWords(echo, 'en', brief({ last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0, not_sharing: 0, unreadable: 0, missed_unreadable: 0, missed_off: 4 } }));
    expect(offOnly.status).toBe(`coachHome.brief.status_empty{"time":"${localTime(RAN_AT)}"} · coachHome.brief.missed_off{"count":4}`);
    const unreadOnly = briefStatusWords(echo, 'en', brief({ last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0, not_sharing: 1, unreadable: 0, missed_unreadable: 1, missed_off: 0 } }));
    expect(unreadOnly.status).toBe(
      `coachHome.brief.status_empty_left_out{"time":"${localTime(RAN_AT)}","count":1} · coachHome.brief.missed_unreadable{"count":1}`,
    );
  });

  it('reads the missed-session clauses in the catalogue words, after the run they close', () => {
    const lookup = (key: string): unknown =>
      key.split('.').reduce<unknown>((node, part) => (node !== null && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined), en);
    const t = (key: string, options?: Record<string, unknown>) =>
      String(lookup(key) ?? key).replace(/\{\{(\w+)\}\}/g, (_m, name: string) => String(options?.[name] ?? ''));
    const words = briefStatusWords(t, 'en', brief({ last_run: { ran_at: RAN_AT, outcome: 'posted', items: 1, not_sharing: 0, unreadable: 0, missed_unreadable: 1, missed_off: 2 } }));
    expect(words.status.endsWith(' · missed sessions not checked for 1: a calendar couldn’t be read · missed sessions not checked for 2: switched off on their side')).toBe(true);
  });
});

describe('isCoachBrief', () => {
  it('takes a brief, with or without a last run', () => {
    expect(isCoachBrief(brief())).toBe(true);
    expect(isCoachBrief(brief({ state: 'paused', last_run: { ran_at: RAN_AT, outcome: 'skipped', items: 0, not_sharing: 0, unreadable: 0, missed_unreadable: 0, missed_off: 0 } }))).toBe(true);
  });

  it('refuses what a fallback page, an older server or a newer outcome answers', () => {
    expect(isCoachBrief('<!doctype html><html></html>')).toBe(false);
    expect(isCoachBrief(null)).toBe(false);
    expect(isCoachBrief({})).toBe(false);
    expect(isCoachBrief({ ...brief(), state: 'sleeping' })).toBe(false);
    expect(isCoachBrief({ ...brief(), muted: undefined })).toBe(false);
    expect(isCoachBrief({ ...brief(), last_run: undefined })).toBe(false);
    expect(isCoachBrief({ ...brief(), last_run: { ran_at: RAN_AT, outcome: 'exploded', items: 0, not_sharing: 0, unreadable: 0, missed_unreadable: 0, missed_off: 0 } })).toBe(false);
    expect(isCoachBrief({ ...brief(), last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0 } })).toBe(false);
    expect(isCoachBrief({ ...brief(), last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0, not_sharing: 1 } })).toBe(false);
    expect(isCoachBrief({ ...brief(), last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0, not_sharing: 1, unreadable: 0 } })).toBe(false);
    expect(isCoachBrief({ ...brief(), last_run: { ran_at: RAN_AT, outcome: 'empty', items: 0, not_sharing: 1, unreadable: 0, missed_unreadable: 0 } })).toBe(false);
  });
});

describe('the hours', () => {
  it('runs from 05:00 to 23:00, one per hour', () => {
    expect(BRIEF_HOURS[0]).toBe(5);
    expect(BRIEF_HOURS[BRIEF_HOURS.length - 1]).toBe(23);
    expect(BRIEF_HOURS).toHaveLength(19);
    expect(BRIEF_HOURS.map(briefHourLabel).slice(0, 2)).toEqual(['05:00', '06:00']);
    expect(briefHourLabel(23)).toBe('23:00');
  });
});

describe('the lists a change replaces', () => {
  it('leaves one athlete out and puts them back, every other id travelling unchanged', () => {
    expect(withMuted(['u-a', 'u-gone'], 'u-b', true)).toEqual(['u-a', 'u-gone', 'u-b']);
    expect(withMuted(['u-a', 'u-b', 'u-gone'], 'u-b', false)).toEqual(['u-a', 'u-gone']);
    // Muting twice lists the athlete once; unmuting someone not listed changes nothing.
    expect(withMuted(['u-a'], 'u-a', true)).toEqual(['u-a']);
    expect(withMuted(['u-a'], 'u-z', false)).toEqual(['u-a']);
  });

  it('says whether the brief leaves an athlete out', () => {
    expect(isBriefMuted(brief({ muted: ['u-a'] }), 'u-a')).toBe(true);
    expect(isBriefMuted(brief({ muted: ['u-a'] }), 'u-b')).toBe(false);
  });

  it('turns a reason off and back on, in the order the settings draw them', () => {
    expect(withReason([], 'plan_ended', false)).toEqual(['plan_ended']);
    expect(withReason(['plan_ended'], 'nothing_received', false)).toEqual(['nothing_received', 'plan_ended']);
    expect(withReason(['nothing_received', 'plan_ended'], 'plan_ended', true)).toEqual(['nothing_received']);
  });

  it('keeps a reason this client does not know switched off', () => {
    const unknown = 'future_kind' as unknown as (typeof BRIEF_REASON_KINDS)[number];
    expect(withReason([unknown], 'plan_ended', false)).toEqual(['plan_ended', unknown]);
  });

  it('draws one switch per reason the brief checks, never the stop it only counts', () => {
    expect([...BRIEF_REASON_KINDS].sort()).toEqual(
      [
        'deep_fatigue_streak',
        'new_member_no_plan',
        'nothing_received',
        'plan_draft_waiting',
        'plan_ended',
        'planned_sessions_missed',
        'provider_reconnect',
        'unverified_figure',
      ],
    );
  });

  it('draws the missed-session switch after the fatigue streak and before the plan states, as the queue ranks them', () => {
    const at = (kind: (typeof BRIEF_REASON_KINDS)[number]) => BRIEF_REASON_KINDS.indexOf(kind);
    expect(at('planned_sessions_missed')).toBe(at('deep_fatigue_streak') + 1);
    expect(at('plan_draft_waiting')).toBe(at('planned_sessions_missed') + 1);
    expect(withReason([], 'planned_sessions_missed', false)).toEqual(['planned_sessions_missed']);
    expect(withReason(['plan_ended'], 'planned_sessions_missed', false)).toEqual(['planned_sessions_missed', 'plan_ended']);
  });

  it('lists the left-out athletes the roster names, in the order they were left out', () => {
    const roster = [
      { user_id: 'u-a', name: 'Alice' },
      { user_id: 'u-b', name: 'Bruno' },
    ];
    expect(mutedAthletes(['u-b', 'u-gone', 'u-a'], roster).map((a) => a.name)).toEqual(['Bruno', 'Alice']);
    expect(mutedAthletes([], roster)).toEqual([]);
  });

  it('shows a change as the server will answer it', () => {
    const before = brief({ reasons_off: ['plan_ended'], muted: ['u-a'] });
    expect(applyBriefUpdate(before, { paused: true }).state).toBe('paused');
    expect(applyBriefUpdate(brief({ state: 'paused' }), { paused: false }).state).toBe('active');
    expect(applyBriefUpdate(before, { hour: 9 })).toEqual({ ...before, hour: 9 });
    expect(applyBriefUpdate(before, { muted: [] })).toEqual({ ...before, muted: [] });
    expect(applyBriefUpdate(before, { reasons_off: [] })).toEqual({ ...before, reasons_off: [] });
    expect(applyBriefUpdate(before, {})).toEqual(before);
  });
});

describe('createCoachBriefHooks', () => {
  const getBrief = vi.fn<() => Promise<CoachBrief>>();
  const updateBrief = vi.fn<(update: CoachBriefUpdate) => Promise<CoachBrief>>();
  const markBriefOpened = vi.fn<(athleteId: string) => Promise<void>>();
  const hooks = createCoachBriefHooks({ getBrief, updateBrief, markBriefOpened });

  function setup() {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => createElement(QueryClientProvider, { client }, children);
    return { client, wrapper };
  }

  beforeEach(() => {
    getBrief.mockReset();
    updateBrief.mockReset();
    markBriefOpened.mockReset();
  });

  it('has no brief for a caller the server refuses, and does not ask again', async () => {
    getBrief.mockRejectedValue({ response: { status: 403 } });
    const { wrapper } = setup();
    const { result } = renderHook(() => hooks.useCoachBrief(true), { wrapper });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.brief).toBeUndefined();
    expect(getBrief).toHaveBeenCalledTimes(1);
  });

  it('has no brief when the read answers something that is not one', async () => {
    getBrief.mockResolvedValue('<!doctype html><html></html>' as unknown as CoachBrief);
    const { wrapper } = setup();
    const { result } = renderHook(() => hooks.useCoachBrief(true), { wrapper });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.brief).toBeUndefined();
  });

  it('shows a mute before the server answers, sending the whole list, then re-reads the brief', async () => {
    getBrief.mockResolvedValue(brief({ muted: ['u-a'] }));
    let answer: (value: CoachBrief) => void = () => {};
    updateBrief.mockImplementation(() => new Promise<CoachBrief>((resolve) => (answer = resolve)));
    const { wrapper } = setup();
    const { result } = renderHook(() => ({ read: hooks.useCoachBrief(true), write: hooks.useUpdateCoachBrief() }), { wrapper });
    await waitFor(() => expect(result.current.read.brief).toBeDefined());

    let done: Promise<void> = Promise.resolve();
    act(() => {
      done = result.current.write.update({ muted: withMuted(result.current.read.brief?.muted ?? [], 'u-b', true) });
    });
    await waitFor(() => expect(result.current.read.brief?.muted).toEqual(['u-a', 'u-b']));
    expect(updateBrief).toHaveBeenCalledWith({ muted: ['u-a', 'u-b'] });

    getBrief.mockResolvedValue(brief({ muted: ['u-a', 'u-b'] }));
    await act(async () => {
      answer(brief({ muted: ['u-a', 'u-b'] }));
      await done;
    });
    await waitFor(() => expect(getBrief).toHaveBeenCalledTimes(2));
    expect(result.current.read.brief?.muted).toEqual(['u-a', 'u-b']);
  });

  it('rolls a refused change back to what the server holds', async () => {
    getBrief.mockResolvedValue(brief({ hour: 7 }));
    updateBrief.mockRejectedValue(new Error('refused'));
    const { wrapper } = setup();
    const { result } = renderHook(() => ({ read: hooks.useCoachBrief(true), write: hooks.useUpdateCoachBrief() }), { wrapper });
    await waitFor(() => expect(result.current.read.brief).toBeDefined());

    await act(async () => {
      await expect(result.current.write.update({ hour: 9 })).rejects.toThrow('refused');
    });
    await waitFor(() => expect(result.current.read.brief?.hour).toBe(7));
  });

  it('pings once per open, never for none, and again when the athlete is opened anew', async () => {
    markBriefOpened.mockResolvedValue(undefined);
    const { wrapper } = setup();
    const { rerender } = renderHook(({ id }: { id: string | null }) => hooks.useBriefOpenedPing(id), {
      wrapper,
      initialProps: { id: null as string | null },
    });
    expect(markBriefOpened).not.toHaveBeenCalled();

    rerender({ id: 'u-a' });
    rerender({ id: 'u-a' });
    await waitFor(() => expect(markBriefOpened).toHaveBeenCalledTimes(1));
    expect(markBriefOpened).toHaveBeenCalledWith('u-a');

    rerender({ id: 'u-b' });
    await waitFor(() => expect(markBriefOpened).toHaveBeenCalledTimes(2));
    rerender({ id: null });
    rerender({ id: 'u-b' });
    await waitFor(() => expect(markBriefOpened).toHaveBeenCalledTimes(3));
  });

  it('swallows a failed ping, thrown or rejected', async () => {
    markBriefOpened.mockRejectedValueOnce(new Error('offline'));
    markBriefOpened.mockImplementationOnce(() => {
      throw new TypeError('not bound');
    });
    const { wrapper } = setup();
    const { rerender } = renderHook(({ id }: { id: string | null }) => hooks.useBriefOpenedPing(id), {
      wrapper,
      initialProps: { id: 'u-a' as string | null },
    });
    rerender({ id: 'u-b' });
    await waitFor(() => expect(markBriefOpened).toHaveBeenCalledTimes(2));
  });
});

describe('the catalogue behind the brief', () => {
  const CATALOGUES = { en, fr, de, es, pt } as const;
  const lookup = (catalogue: unknown, key: string): unknown =>
    key.split('.').reduce<unknown>((node, part) => (node !== null && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined), catalogue);

  const keys = [
    ...[
      'title',
      'schedule',
      'status_never',
      'status_empty',
      'status_empty_left_out',
      'status_posted',
      'status_held',
      'status_paused',
      'missed_unreadable',
      'missed_off',
      'pause',
      'resume',
      'settings',
      'hour_label',
      'reasons_label',
      'muted_label',
      'mute',
      'unmute',
      'muted_none',
      'save_failed',
    ].map((key) => `coachHome.brief.${key}`),
    ...Object.values(COACH_BRIEF_REASON_LABEL_KEY),
  ];

  for (const [locale, catalogue] of Object.entries(CATALOGUES)) {
    it(`${locale} names every key the brief reads`, () => {
      expect(keys.filter((key) => typeof lookup(catalogue, key) !== 'string')).toEqual([]);
    });
  }
});
