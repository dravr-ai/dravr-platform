// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home's three phone views over a mocked roster — À traiter, the two-line team list and the season countdown
// ABOUTME: Pins the server's order in the queue, days-without-activity (never form) in the list, non-sharers as status only, and every state of the roster read

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachRosterResponse } from '@pierre/shared-types';
import {
  CAMILLE,
  FELIX,
  HUGO,
  JULIEN,
  LEA,
  MAUDE,
  OLIVIER,
  ROSTER,
  day,
  nonSharer,
} from '../integration/app/helpers/coachRosterFixtures';

const mockPush = jest.fn();
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
    // The router's contract: run on focus; the screen skips the first.
    useFocusEffect: (callback: () => void) => {
      const React = require('react');
      React.useEffect(() => callback(), [callback]);
    },
  }),
);

const mockGetRoster = jest.fn<Promise<CoachRosterResponse>, [{ offset?: number; limit?: number }?]>();
const mockGetConversations = jest.fn();
jest.mock('../src/services/api', () => ({
  coachRosterApi: {
    getRoster: (query?: { offset?: number; limit?: number }) => mockGetRoster(query),
    dismissAttention: jest.fn(),
    restoreAttention: jest.fn(),
  },
  chatApi: {
    getConversations: (limit?: number, offset?: number) => mockGetConversations(limit, offset),
    openAthleteThread: jest.fn(),
  },
}));

import { CoachHomeScreen } from '../src/screens/coach-home/CoachHomeScreen';
import { coachAthleteHref, coachRepliesHref } from '../src/navigation/routes';

function renderCoachHome() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <CoachHomeScreen scopeSwitch={null} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  mockGetRoster.mockResolvedValue(ROSTER);
  mockGetConversations.mockResolvedValue({ conversations: [], total: 0 });
});

describe('the roster read', () => {
  it('shows a spinner, then the queue with the scope line', async () => {
    const screen = renderCoachHome();
    expect(screen.getByTestId('coach-home-loading')).toBeTruthy();
    await screen.findByTestId('coach-queue');
    expect(screen.getByTestId('coach-scope').props.children).toContain('8');
    expect(mockGetRoster).toHaveBeenCalledWith({ offset: 0, limit: 100 });
  });

  it('opens the reply review on every athlete from the Replies row', async () => {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    fireEvent.press(screen.getByTestId('coach-replies-link'));
    expect(mockPush).toHaveBeenCalledWith(coachRepliesHref());
    expect(coachRepliesHref()).toEqual({ pathname: '/(app)/(tabs)/(home)/replies', params: {} });
  });

  it('says it could not load, and retries', async () => {
    mockGetRoster.mockRejectedValueOnce(new Error('down'));
    const screen = renderCoachHome();
    await screen.findByTestId('coach-home-failed');
    fireEvent.press(screen.getByTestId('coach-home-retry'));
    await screen.findByTestId('coach-queue');
    expect(mockGetRoster).toHaveBeenCalledTimes(2);
  });

  it('says so when the coach holds no athlete', async () => {
    mockGetRoster.mockResolvedValue({ ...ROSTER, athletes: [], scope: { visible_members: 0, roster_members: 0 } });
    const screen = renderCoachHome();
    await screen.findByTestId('coach-home-empty');
  });

  it('follows next_offset until the last page and merges them', async () => {
    mockGetRoster
      .mockResolvedValueOnce({ ...ROSTER, athletes: [FELIX], next_offset: 1 })
      .mockResolvedValueOnce({ ...ROSTER, athletes: [JULIEN], next_offset: null });
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    expect(mockGetRoster).toHaveBeenNthCalledWith(2, { offset: 1, limit: 100 });
    expect(screen.getByTestId('coach-queue-row-felix')).toBeTruthy();
    expect(screen.getByTestId('coach-queue-row-julien')).toBeTruthy();
  });
});

describe('À traiter', () => {
  it('lists today then this week, each by the server rank', async () => {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    const today = within(screen.getByTestId('coach-queue-today'));
    const week = within(screen.getByTestId('coach-queue-week'));
    // Maude rank 10, Félix 20, Julien 30, Léa 40: the server's order, not alphabetical.
    expect(today.getAllByTestId(/^coach-queue-row-/).map((n) => n.props.testID)).toEqual([
      'coach-queue-row-maude',
      'coach-queue-row-felix',
      'coach-queue-row-julien',
      'coach-queue-row-lea',
    ]);
    expect(week.getAllByTestId(/^coach-queue-row-/).map((n) => n.props.testID)).toEqual(['coach-queue-row-mathieu']);
    expect(screen.getByTestId('coach-queue-count').props.children).toBe(5);
  });

  it('puts a non-sharer who revoked on today, with the reason in words and a quiet avatar', async () => {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    // Maude carries consent_revoked at rank 10: she heads today.
    const first = within(screen.getByTestId('coach-queue-today')).getAllByTestId(/^coach-queue-row-/)[0];
    expect(first.props.testID).toBe('coach-queue-row-maude');
    expect(within(first).getByTestId('coach-queue-reason').props.children).toMatch(/no longer shares/i);
  });

  it('marks today with a filled dot and the week with a hollow one', async () => {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    const hidden = { hidden: true };
    expect(within(screen.getByTestId('coach-queue-row-felix')).getByTestId('coach-queue-horizon-today', hidden)).toBeTruthy();
    expect(within(screen.getByTestId('coach-queue-row-mathieu')).getByTestId('coach-queue-horizon-week', hidden)).toBeTruthy();
  });

  it('closes on one line of names for those with nothing to flag, and those who do not share', async () => {
    mockGetRoster.mockResolvedValue({
      ...ROSTER,
      athletes: [FELIX, CAMILLE, OLIVIER, nonSharer('noe', 'Noé Roy', { attention: [] })],
    });
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    const calm = screen.getByTestId('coach-queue-calm-line');
    expect(calm).toHaveTextContent(/Nothing to flag · 2/);
    expect(calm).toHaveTextContent(/Camille Bouchard, Olivier Fortin/);
    expect(screen.getByTestId('coach-queue-not-sharing')).toHaveTextContent(/Noé Roy/);
  });

  it('opens the athlete on a tap', async () => {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    fireEvent.press(screen.getByTestId('coach-queue-row-julien'));
    expect(mockPush).toHaveBeenCalledWith(coachAthleteHref('julien'));
  });

  it('says there is nothing to treat when no reason is open', async () => {
    mockGetRoster.mockResolvedValue({ ...ROSTER, athletes: [CAMILLE, OLIVIER] });
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue-empty');
  });

  it('lists a treated athlete under Treated with an undo that restores the reason', async () => {
    const treated = {
      ...FELIX,
      attention: FELIX.attention.map((r) => ({ ...r, dismissed_at: `${day(0)}T09:00:00Z` })),
    };
    mockGetRoster.mockResolvedValue({ ...ROSTER, athletes: [treated, JULIEN] });
    const restore = jest.requireMock('../src/services/api').coachRosterApi.restoreAttention as jest.Mock;
    restore.mockResolvedValue({ reason_id: treated.attention[0].id, restored: true });
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue-treated');
    expect(screen.queryByTestId('coach-queue-row-felix')).toBeNull();
    expect(screen.getByTestId('coach-queue-count').props.children).toBe(1);
    fireEvent.press(screen.getByTestId('coach-treated-undo-felix'));
    await waitFor(() => expect(restore).toHaveBeenCalledWith({ athlete_user_id: 'felix', reason_id: treated.attention[0].id }));
    // Optimistic: Félix is back in the queue before any refetch.
    await screen.findByTestId('coach-queue-row-felix');
  });
});

describe('the team list', () => {
  async function openRoster() {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    fireEvent.press(screen.getByTestId('coach-views-roster'));
    await screen.findByTestId('coach-roster');
    return screen;
  }

  it('is reached from the tabs and from "See all", and is sorted by days without activity', async () => {
    const screen = await openRoster();
    const rows = within(screen.getByTestId('coach-roster-list')).getAllByTestId(/^coach-row-[a-z]+$/);
    // Julien 6 days, Olivier 3, Camille 2, then the 0-1 day ones by name; none by form.
    const ids = rows.map((n) => String(n.props.testID).replace('coach-row-', ''));
    expect(ids.slice(0, 3)).toEqual(['julien', 'olivier', 'camille']);
    // Léa and Thomas trained a day ago, Émilie, Félix and Mathieu today; ties by name. Léa's deep fatigue buys her no place.
    expect(ids.slice(3)).toEqual(['lea', 'thomas', 'emilie', 'felix', 'mathieu']);
    expect(screen.getByTestId('coach-roster-sort')).toHaveTextContent(/days without activity/i);
  });

  it('opens from "See all" in the queue', async () => {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    fireEvent.press(screen.getByTestId('coach-see-all'));
    await screen.findByTestId('coach-roster');
  });

  it('draws each sharer on two lines: name with the race countdown, then the band strip and words', async () => {
    const screen = await openRoster();
    const julien = within(screen.getByTestId('coach-row-julien'));
    // A cut reading is dated and its strip drawn quieter.
    expect(julien.getByTestId('form-strip').props.accessibilityLabel).toMatch(/Balanced/i);
    expect(screen.getByTestId('coach-row-line-julien')).toHaveTextContent(/Balanced, as of .* · 6 d ago/i);
    expect(julien.getByTestId('coach-row-race-julien')).toHaveTextContent('D-25');
    // The band is lit on one of six steps; insufficient history lights none.
    expect(julien.getAllByTestId('form-strip-lit')).toHaveLength(1);
    expect(within(screen.getByTestId('coach-row-camille')).queryByTestId('form-strip-lit')).toBeNull();
    expect(screen.getByTestId('coach-row-line-camille')).toHaveTextContent(/Not enough history/i);
  });

  it('prints no form number and no ranking across athletes', async () => {
    const screen = await openRoster();
    // The footnote says no team average is computed; no figure of form is printed.
    expect(screen.queryByText(/TSB|\d\s?%/)).toBeNull();
  });

  it('keeps non-sharers apart with a status line and nothing of their training', async () => {
    const screen = await openRoster();
    const block = within(screen.getByTestId('coach-roster-non-sharers'));
    expect(block.getByTestId('coach-nonsharer-maude')).toHaveTextContent(/stopped sharing on/i);
    expect(block.getByTestId('coach-nonsharer-hugo')).toHaveTextContent(/any data source/i);
    expect(block.queryByTestId('form-strip')).toBeNull();
    // They are not among the sharers' rows.
    expect(within(screen.getByTestId('coach-roster-list')).queryByTestId('coach-row-maude')).toBeNull();
    expect(MAUDE.training).toBeUndefined();
    expect(HUGO.training).toBeUndefined();
  });

  it('writes to a non-sharer in their group room, or the Chat list with none open', async () => {
    const screen = await openRoster();
    fireEvent.press(screen.getByTestId('coach-nonsharer-write-maude'));
    expect(mockPush).toHaveBeenCalledWith('/(app)/(tabs)/(chat)');
  });

  it('opens the room of the group when the coach holds one', async () => {
    mockGetConversations.mockResolvedValue({
      conversations: [{ id: 'room-tri', group_id: 'g-tri', title: 'Tri', message_count: 0, created_at: '', updated_at: '' }],
      total: 1,
    });
    const screen = await openRoster();
    await waitFor(() => expect(mockGetConversations).toHaveBeenCalled());
    await waitFor(() => {
      fireEvent.press(screen.getByTestId('coach-nonsharer-write-maude'));
      expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/chat/[conversationId]', params: { conversationId: 'room-tri' } });
    });
  });

  it('filters by group on the tabs, with each group counting its athletes', async () => {
    mockGetRoster.mockResolvedValue({
      ...ROSTER,
      groups: [
        { id: 'g-tri', name: 'Tri', athlete_count: 2 },
        { id: 'g-route', name: 'Route', athlete_count: 1 },
      ],
      athletes: [
        { ...FELIX },
        { ...JULIEN },
        { ...LEA, groups: [{ id: 'g-route', name: 'Route' }] },
      ],
    });
    const screen = await openRoster();
    expect(screen.getByTestId('coach-group-tabs-all')).toHaveTextContent('All 3');
    fireEvent.press(screen.getByTestId('coach-group-tabs-g-route'));
    expect(screen.getByTestId('coach-roster-count').props.children).toBe(1);
    expect(screen.getByTestId('coach-row-lea')).toBeTruthy();
    expect(screen.queryByTestId('coach-row-felix')).toBeNull();
  });

  it('opens an athlete on a tap', async () => {
    const screen = await openRoster();
    fireEvent.press(screen.getByTestId('coach-row-olivier'));
    expect(mockPush).toHaveBeenCalledWith(coachAthleteHref('olivier'));
  });
});

describe('the season countdown', () => {
  async function openSeason() {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    fireEvent.press(screen.getByTestId('coach-views-season'));
    await screen.findByTestId('coach-season');
    return screen;
  }

  it('lists A races ahead by date, soonest first, each with its countdown and phase bars', async () => {
    const screen = await openSeason();
    const ahead = within(screen.getByTestId('coach-season-ahead'));
    const ids = ahead.getAllByTestId(/^coach-season-row-/).map((n) => String(n.props.testID).replace('coach-season-row-', ''));
    // Félix and Julien 25 days, Léa 39, Émilie 165; ties by name.
    expect(ids).toEqual(['felix', 'julien', 'lea', 'emilie']);
    expect(ahead.getByTestId('coach-season-countdown-felix')).toHaveTextContent('D-25');
    expect(ahead.getByTestId('coach-season-countdown-emilie')).toHaveTextContent('D-165');
    expect(ahead.getByTestId('coach-season-bars-felix', { hidden: true })).toBeTruthy();
  });

  it('puts those with no A race under their own heading with the state that says why', async () => {
    const screen = await openSeason();
    const without = within(screen.getByTestId('coach-season-without'));
    const olivier = without.getByTestId('coach-season-row-olivier');
    expect(within(olivier).getByTestId('coach-season-subline')).toHaveTextContent(/Nothing planned since/i);
    expect(without.getByTestId('coach-season-row-thomas')).toHaveTextContent(/No plan yet/i);
    expect(within(without.getByTestId('coach-season-row-thomas')).getByTestId('coach-season-subline')).toBeTruthy();
  });

  it('says a stale source plainly, and names those whose plans are not shown', async () => {
    const screen = await openSeason();
    expect(within(screen.getByTestId('coach-season-row-julien')).getByText(/no data since/i)).toBeTruthy();
    const hidden = screen.getByTestId('coach-season-hidden');
    expect(hidden).toHaveTextContent(/Hugo Lemieux/);
    expect(hidden).toHaveTextContent(/Maude Poirier/);
  });

  it('prints no form in the season', async () => {
    const screen = await openSeason();
    expect(screen.queryByTestId('form-strip')).toBeNull();
  });

  it('opens an athlete on a tap', async () => {
    const screen = await openSeason();
    fireEvent.press(screen.getByTestId('coach-season-row-lea'));
    expect(mockPush).toHaveBeenCalledWith(coachAthleteHref('lea'));
  });
});
