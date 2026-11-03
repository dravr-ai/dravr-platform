// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home's three phone views over a mocked roster — À traiter, the two-line team list and the season countdown
// ABOUTME: Pins the server's order in the queue, days-without-activity (never form) in the list, non-sharers as status only, and every state of the roster read

import React from 'react';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
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
const mockOpenGroupRoom = jest.fn();
jest.mock('../src/services/api', () => ({
  coachRosterApi: {
    getRoster: (query?: { offset?: number; limit?: number }) => mockGetRoster(query),
    dismissAttention: jest.fn(),
    restoreAttention: jest.fn(),
  },
  chatApi: {
    openGroupRoom: (groupId: string) => mockOpenGroupRoom(groupId),
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
  mockOpenGroupRoom.mockReset();
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
    expect(screen.getByTestId('coach-views-queue-count').props.children).toBe(5);
  });

  it('names "To handle" once: the tab carries the count, the queue no second title', async () => {
    const screen = renderCoachHome();
    await screen.findByTestId('coach-queue');
    expect(screen.getAllByText('To handle')).toHaveLength(1);
    expect(within(screen.getByTestId('coach-views-queue')).getByText('To handle')).toBeTruthy();
    expect(screen.queryByTestId('coach-queue-count')).toBeNull();
    expect(screen.getByTestId('coach-see-all')).toBeTruthy();
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
    // Each name is its own link, a row-high target the test driver reaches by id.
    const names = within(calm).getAllByRole('link');
    expect(names.map((n) => n.props.testID)).toEqual(['coach-calm-camille', 'coach-calm-olivier']);
    expect(within(calm).getByTestId('coach-calm-camille')).toHaveTextContent('Camille Bouchard');
    const notSharing = screen.getByTestId('coach-queue-not-sharing');
    expect(notSharing).toHaveTextContent(/Not sharing:/);
    fireEvent.press(within(notSharing).getByTestId('coach-not-sharing-noe'));
    expect(mockPush).toHaveBeenCalledWith(coachAthleteHref('noe'));
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
    expect(screen.getByTestId('coach-views-queue-count').props.children).toBe(1);
    // Name over reason, as the open rows: the reason is not squeezed onto the name's line.
    const treatedRow = within(screen.getByTestId('coach-treated-row-felix'));
    expect(treatedRow.getByText('Félix Morin')).toBeTruthy();
    expect(treatedRow.getByTestId('coach-treated-reason')).not.toHaveTextContent(/Félix/);
    // The undo is its own row-high target with a side pad, not a word with a slop beside the athlete.
    const undoTarget = screen.getByTestId('coach-treated-undo-felix');
    expect(undoTarget.props.className).toMatch(/min-w-11/);
    expect(undoTarget.props.className).toMatch(/px-4/);
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
    // A status that wraps keeps its dot beside the first word.
    const statusRow = block.getByTestId('coach-nonsharer-status-maude').parent?.parent;
    expect(statusRow?.props.className).toMatch(/items-start/);
    expect(block.queryByTestId('form-strip')).toBeNull();
    // They are not among the sharers' rows.
    expect(within(screen.getByTestId('coach-roster-list')).queryByTestId('coach-row-maude')).toBeNull();
    expect(MAUDE.training).toBeUndefined();
    expect(HUGO.training).toBeUndefined();
  });

  it('writes to a non-sharer in their group room, as the server finds or opens it', async () => {
    mockOpenGroupRoom.mockResolvedValue({ id: 'room-tri', group_id: 'g-tri' });
    const screen = await openRoster();
    // One action, named after the place it opens.
    expect(screen.getByTestId('coach-nonsharer-write-maude')).toHaveTextContent('Write in the group');
    expect(screen.queryByText('Message them')).toBeNull();
    fireEvent.press(screen.getByTestId('coach-nonsharer-write-maude'));
    await waitFor(() => expect(mockOpenGroupRoom).toHaveBeenCalledWith('g-tri'));
    await waitFor(() =>
      expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/chat/[conversationId]', params: { conversationId: 'room-tri' } }),
    );
    expect(mockPush).not.toHaveBeenCalledWith('/(app)/(tabs)/(chat)');
  });

  it('opens one room however often the link is tapped while it opens', async () => {
    let resolve: (room: unknown) => void = () => undefined;
    mockOpenGroupRoom.mockReturnValue(new Promise((r) => (resolve = r)));
    const screen = await openRoster();
    fireEvent.press(screen.getByTestId('coach-nonsharer-write-maude'));
    fireEvent.press(screen.getByTestId('coach-nonsharer-write-maude'));
    await waitFor(() => expect(screen.getByTestId('coach-nonsharer-write-maude')).toHaveTextContent('Opening the group…'));
    expect(mockOpenGroupRoom).toHaveBeenCalledTimes(1);
    await act(async () => resolve({ id: 'room-tri', group_id: 'g-tri' }));
    expect(mockPush).toHaveBeenCalledTimes(1);
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
    expect(within(olivier).getByTestId('coach-season-subline')).toHaveTextContent(/Nothing planned after/i);
    expect(without.getByTestId('coach-season-row-thomas')).toHaveTextContent(/No plan yet/i);
    expect(within(without.getByTestId('coach-season-row-thomas')).getByTestId('coach-season-subline')).toBeTruthy();
  });

  it('says a stale source plainly, and names those whose plans are not shown', async () => {
    const screen = await openSeason();
    expect(within(screen.getByTestId('coach-season-row-julien')).getByText(/no data since/i)).toBeTruthy();
    const hidden = within(screen.getByTestId('coach-season-hidden'));
    // A label with its count, so it checks against the scope line, and each one's status.
    expect(hidden.getByText('Not sharing')).toBeTruthy();
    expect(hidden.getByText('2')).toBeTruthy();
    expect(hidden.getByTestId('coach-season-hidden-hugo')).toHaveTextContent(/Hugo Lemieux · Hasn.t connected any data source/);
    expect(hidden.getByTestId('coach-season-hidden-maude')).toHaveTextContent(/Maude Poirier · Stopped sharing on/);
  });

  it('dates a plan that ran out by its last planned day, as the queue does', async () => {
    const screen = await openSeason();
    // Olivier's season `ends` is the day after his plan's last day.
    const lastDay = new Date(`${day(-13)}T12:00:00`).toLocaleDateString('en', { day: 'numeric', month: 'short' });
    const subline = within(screen.getByTestId('coach-season-row-olivier')).getByTestId('coach-season-subline');
    expect(subline).toHaveTextContent(new RegExp(`${lastDay}$`));
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
