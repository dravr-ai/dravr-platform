// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A roster athlete's detail on the phone — why now with its evidence, the form strip, plan, sources, the agent's week, and the actions
// ABOUTME: Pins "Traité" as optimistic with a rollback and an alert on refusal, its undo, and that an action without a phone destination is not offered

import React from 'react';
import { Alert } from 'react-native';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachRosterAthlete, CoachRosterResponse } from '@pierre/shared-types';
import { HUGO, JULIEN, LEA, MATHIEU, ROSTER } from '../integration/app/helpers/coachRosterFixtures';

const mockPush = jest.fn();
let mockAthleteId = 'lea';
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
    useLocalSearchParams: () => ({ athleteId: mockAthleteId }),
  }),
);

const mockGetRoster = jest.fn<Promise<CoachRosterResponse>, [unknown?]>();
const mockDismiss = jest.fn();
const mockRestore = jest.fn();
const mockOpenThread = jest.fn();
jest.mock('../src/services/api', () => ({
  coachRosterApi: {
    getRoster: (query?: unknown) => mockGetRoster(query),
    dismissAttention: (request: unknown) => mockDismiss(request),
    restoreAttention: (request: unknown) => mockRestore(request),
  },
  chatApi: {
    getConversations: jest.fn().mockResolvedValue({ conversations: [], total: 0 }),
    openAthleteThread: (athleteId: string, groupId: string) => mockOpenThread(athleteId, groupId),
  },
}));

import { CoachAthleteScreen } from '../src/screens/coach-home/CoachAthleteScreen';

function renderAthlete(id: string, athletes: CoachRosterAthlete[] = ROSTER.athletes) {
  mockAthleteId = id;
  mockGetRoster.mockResolvedValue({ ...ROSTER, athletes });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <CoachAthleteScreen />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
});

describe('why now', () => {
  it('names the reason and each line of evidence with its source and time', async () => {
    const screen = renderAthlete('lea');
    const why = within(await screen.findByTestId('coach-why'));
    expect(why.getByText(/Deep fatigue for 3 mornings/i)).toBeTruthy();
    const line = why.getByTestId('coach-evidence');
    expect(line).toHaveTextContent(/34/);
    expect(within(line).getByTestId('coach-evidence-source')).toHaveTextContent(/Garmin · .*· /);
  });

  it('lists the other open reasons under the lead one', async () => {
    const both = { ...JULIEN, attention: [...JULIEN.attention, { ...MATHIEU.attention[0], id: 'plan_draft_waiting' }] };
    const screen = renderAthlete('julien', [both]);
    const why = within(await screen.findByTestId('coach-why'));
    expect(why.getByTestId('coach-reason-nothing_received')).toBeTruthy();
    expect(why.getByTestId('coach-reason-plan_draft_waiting')).toBeTruthy();
  });

  it('says a calm sharer needs nothing', async () => {
    const screen = renderAthlete('camille');
    await screen.findByTestId('coach-calm');
    expect(screen.queryByTestId('coach-treat')).toBeNull();
  });
});

describe('form, plan, sources and the agent', () => {
  it('lights one of six steps for the server band and names the athlete\'s own scale', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-form-block'));
    expect(block.getByTestId('coach-form-word')).toHaveTextContent('Deep fatigue');
    expect(block.getAllByTestId('form-strip-lit')).toHaveLength(1);
    expect(block.getByText(/never .* anyone else|never to/i)).toBeTruthy();
  });

  it('reads insufficient history as its own state, with no step lit', async () => {
    const screen = renderAthlete('camille');
    const block = within(await screen.findByTestId('coach-form-block'));
    expect(block.getByTestId('coach-form-word')).toHaveTextContent(/Not enough history/i);
    expect(block.queryByTestId('form-strip-lit')).toBeNull();
    expect(block.getByText(/Not enough history to read/i)).toBeTruthy();
  });

  it('dates a reading whose data is cut', async () => {
    const screen = renderAthlete('julien');
    const block = within(await screen.findByTestId('coach-form-block'));
    expect(block.getByText(/Read on .* — out of date until the data can be read again/i)).toBeTruthy();
  });

  it('shows the plan phases, today\'s line and the A race countdown', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-plan-block'));
    expect(block.getByText(/Ironman Cozumel/)).toBeTruthy();
    expect(block.getByTestId('coach-countdown')).toHaveTextContent('D-39');
    expect(block.getByTestId('coach-today-line', { hidden: true })).toBeTruthy();
    expect(block.getByText(/Week 3 of 12/)).toBeTruthy();
  });

  it('says there is no plan when there is none', async () => {
    const screen = renderAthlete('camille');
    const block = within(await screen.findByTestId('coach-plan-block'));
    expect(block.getByText(/No plan yet/i)).toBeTruthy();
  });

  it('lists the sources with a dot beside a word, a source to reconnect called so', async () => {
    const screen = renderAthlete('julien');
    const source = within(await screen.findByTestId('coach-source-garmin'));
    expect(source.getByText('Garmin')).toBeTruthy();
    expect(source.getByText(/Reconnect/i)).toBeTruthy();
  });

  it('counts the agent\'s week and flags figures no source backs', async () => {
    const screen = renderAthlete('felix');
    const block = within(await screen.findByTestId('coach-agent-block'));
    expect(block.getByText(/3 replies this week, 2 verified/)).toBeTruthy();
    expect(block.getByTestId('coach-agent-verdict')).toHaveTextContent(/1 unverified figure/i);
  });

  it('says the agent was silent when it was', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-agent-block'));
    expect(block.getByText(/No agent replies in their groups/i)).toBeTruthy();
  });
});

describe('a non-sharer', () => {
  it('shows who they are and a note, and nothing of their training', async () => {
    const screen = renderAthlete('hugo');
    await screen.findByTestId('coach-hidden-note');
    expect(screen.queryByTestId('coach-form-block')).toBeNull();
    expect(screen.queryByTestId('coach-plan-block')).toBeNull();
    expect(screen.queryByTestId('coach-sources-block')).toBeNull();
    expect(HUGO.training).toBeUndefined();
  });
});

describe('actions', () => {
  it('writes to the athlete in the group named by the reason', async () => {
    const screen = renderAthlete('lea');
    fireEvent.press(await screen.findByTestId('coach-primary-action'));
    // The primary action is the group room, so the secondary one would repeat it.
    expect(screen.queryByTestId('coach-write-group')).toBeNull();
    expect(mockPush).toHaveBeenCalledWith('/(app)/(tabs)/(chat)');
  });

  it('opens the coach\'s plan thread on a draft to review', async () => {
    mockOpenThread.mockResolvedValue({ id: 'thread-9' });
    const screen = renderAthlete('mathieu');
    fireEvent.press(await screen.findByTestId('coach-primary-action'));
    await waitFor(() => expect(mockOpenThread).toHaveBeenCalledWith('mathieu', 'g-tri'));
    await waitFor(() =>
      expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/chat/[conversationId]', params: { conversationId: 'thread-9' } }),
    );
  });

  it('says why a plan thread was refused instead of doing nothing', async () => {
    mockOpenThread.mockRejectedValue(new Error('refused'));
    const screen = renderAthlete('mathieu');
    fireEvent.press(await screen.findByTestId('coach-primary-action'));
    await waitFor(() => expect(Alert.alert).toHaveBeenCalled());
    expect(mockPush).not.toHaveBeenCalled();
  });

  it('opens the reply review on the athlete from "Correct the reply", beside the group\'s room', async () => {
    const screen = renderAthlete('felix');
    fireEvent.press(await screen.findByTestId('coach-primary-action'));
    expect(screen.getByText(/Correct the reply/i)).toBeTruthy();
    expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/(tabs)/(home)/replies', params: { athleteId: 'felix' } });
    fireEvent.press(screen.getByTestId('coach-write-group'));
    expect(mockPush).toHaveBeenLastCalledWith('/(app)/(tabs)/(chat)');
  });

  it('opens the reply review on the athlete from the agent block', async () => {
    const screen = renderAthlete('felix');
    fireEvent.press(await screen.findByTestId('coach-agent-replies-link'));
    expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/(tabs)/(home)/replies', params: { athleteId: 'felix' } });
  });

  it('offers no reply review link to an athlete the agent has not answered', async () => {
    const screen = renderAthlete('camille');
    await screen.findByTestId('coach-agent-block');
    expect(screen.queryByTestId('coach-agent-replies-link')).toBeNull();
  });
});

describe('Traité and its undo', () => {
  it('marks the reason handled at once, with the fingerprint it was raised on', async () => {
    let resolve: (v: unknown) => void = () => undefined;
    mockDismiss.mockReturnValue(new Promise((r) => (resolve = r)));
    const screen = renderAthlete('lea');
    fireEvent.press(await screen.findByTestId('coach-treat'));
    await waitFor(() =>
      expect(mockDismiss).toHaveBeenCalledWith({
        athlete_user_id: 'lea',
        reason_id: 'deep_fatigue_streak',
        fingerprint: 'fp-deep_fatigue_streak',
      }),
    );
    // Before the server answers the detail already reads treated, with the undo.
    await screen.findByTestId('coach-treated-note');
    expect(screen.queryByTestId('coach-treat')).toBeNull();
    expect(screen.getByTestId('coach-undo')).toBeTruthy();
    resolve({ reason_id: 'deep_fatigue_streak', fingerprint: 'fp-deep_fatigue_streak', dismissed_at: new Date().toISOString() });
    await screen.findByTestId('coach-undo');
  });

  it('rolls back and says so when the server refuses', async () => {
    mockDismiss.mockRejectedValue(new Error('403'));
    const screen = renderAthlete('lea');
    fireEvent.press(await screen.findByTestId('coach-treat'));
    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith(expect.any(String), expect.stringMatching(/Couldn.t mark it as handled/i)));
    await screen.findByTestId('coach-why');
    expect(screen.getByTestId('coach-treat')).toBeTruthy();
    expect(screen.queryByTestId('coach-undo')).toBeNull();
  });

  it('undoes a treatment and brings the reason back', async () => {
    const treated: CoachRosterAthlete = {
      ...LEA,
      attention: LEA.attention.map((r) => ({ ...r, dismissed_at: new Date().toISOString() })),
    };
    mockRestore.mockResolvedValue({ reason_id: 'deep_fatigue_streak', restored: true });
    const screen = renderAthlete('lea', [treated]);
    fireEvent.press(await screen.findByTestId('coach-undo'));
    await waitFor(() => expect(mockRestore).toHaveBeenCalledWith({ athlete_user_id: 'lea', reason_id: 'deep_fatigue_streak' }));
    await screen.findByTestId('coach-why');
    expect(screen.getByTestId('coach-treat')).toBeTruthy();
  });

  it('rolls an undo back and says so when the server refuses', async () => {
    const treated: CoachRosterAthlete = {
      ...LEA,
      attention: LEA.attention.map((r) => ({ ...r, dismissed_at: new Date().toISOString() })),
    };
    mockRestore.mockRejectedValue(new Error('500'));
    const screen = renderAthlete('lea', [treated]);
    fireEvent.press(await screen.findByTestId('coach-undo'));
    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith(expect.any(String), expect.stringMatching(/Couldn.t undo/i)));
    await screen.findByTestId('coach-treated-note');
  });
});
