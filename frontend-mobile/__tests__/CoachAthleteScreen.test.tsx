// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A roster athlete's detail on the phone — why now with its evidence, the form strip, plan, sources, the agent's week, and the actions
// ABOUTME: Pins "Traité" as optimistic with a rollback and an alert on refusal, its undo, and that an action without a phone destination is not offered

import React from 'react';
import { Alert } from 'react-native';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachRosterAthlete, CoachRosterResponse } from '@pierre/shared-types';
import { HUGO, JULIEN, LEA, MATHIEU, ROSTER, day, reason } from '../integration/app/helpers/coachRosterFixtures';

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
const mockOpenGroupRoom = jest.fn();
jest.mock('../src/services/api', () => ({
  coachRosterApi: {
    getRoster: (query?: unknown) => mockGetRoster(query),
    dismissAttention: (request: unknown) => mockDismiss(request),
    restoreAttention: (request: unknown) => mockRestore(request),
  },
  chatApi: {
    openGroupRoom: (groupId: string) => mockOpenGroupRoom(groupId),
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

/** Lays a phase's unseen label out at `width` points, as the phone draws it in the coach's language. */
function measureLabel(block: ReturnType<typeof within>, kind: string, width: number) {
  fireEvent(block.getByTestId(`coach-phase-measure-${kind}`, { hidden: true }), 'layout', {
    nativeEvent: { layout: { x: 0, y: 0, width, height: 16 } },
  });
}

beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  mockOpenGroupRoom.mockReset();
});

describe('why now', () => {
  it('names the reason and each line of evidence with its source and time', async () => {
    const screen = renderAthlete('lea');
    const why = within(await screen.findByTestId('coach-why'));
    expect(why.getByTestId('coach-reason-label')).toHaveTextContent('Deep fatigue for 3 mornings in a row');
    const line = why.getByTestId('coach-evidence');
    expect(line).toHaveTextContent(/34/);
    expect(within(line).getByTestId('coach-evidence-source')).toHaveTextContent(/Garmin · .*· /);
  });

  it('opens the reason with a full sentence that names the athlete, over its short label', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-reason-deep_fatigue_streak'));
    expect(block.getByTestId('coach-reason-title')).toHaveTextContent('Léa Gagnon is in deep fatigue for the 3rd morning in a row.');
    expect(block.getByTestId('coach-reason-label')).toHaveTextContent('Deep fatigue for 3 mornings in a row');
  });

  it('gives every open reason its own sentence', async () => {
    const both = { ...JULIEN, attention: [...JULIEN.attention, { ...MATHIEU.attention[0], id: 'plan_draft_waiting' }] };
    const screen = renderAthlete('julien', [both]);
    const why = within(await screen.findByTestId('coach-why'));
    const titles = why.getAllByTestId('coach-reason-title').map((node) => String(node.props.children));
    expect(titles).toHaveLength(2);
    expect(titles.every((text) => text.includes(JULIEN.name))).toBe(true);
    expect(within(why.getByTestId('coach-reason-plan_draft_waiting')).getByTestId('coach-reason-title')).toHaveTextContent(
      `The plan drafted with the agent is waiting for your review; ${JULIEN.name} can’t see it yet.`,
    );
  });

  it('lists the other open reasons under the lead one', async () => {
    const both = { ...JULIEN, attention: [...JULIEN.attention, { ...MATHIEU.attention[0], id: 'plan_draft_waiting' }] };
    const screen = renderAthlete('julien', [both]);
    const why = within(await screen.findByTestId('coach-why'));
    expect(why.getByTestId('coach-reason-nothing_received')).toBeTruthy();
    expect(why.getByTestId('coach-reason-plan_draft_waiting')).toBeTruthy();
  });

  it('reads a streak of fatigued mornings as one line, the earlier ones in brackets, under one source', async () => {
    const source = { kind: 'training_load' as const, providers: ['garmin'] };
    const mornings = [0, -1, -2, -3].map((offset, index) => ({
      key: 'coachHome.evidence.formMorning',
      params: { date: day(offset), form_pct: -33 + index },
      source,
      at: `${day(0)}T06:30:00Z`,
    }));
    const streak = { ...LEA, attention: [{ ...LEA.attention[0], evidence: mornings }] };
    const screen = renderAthlete('lea', [streak]);
    const why = within(await screen.findByTestId('coach-why'));
    const lines = why.getAllByTestId('coach-evidence');
    expect(lines).toHaveLength(1);
    expect(lines[0]).toHaveTextContent(/coachHome\.evidence\.formMornings|Form at -33%.*\(.*-30%/);
    expect(why.getAllByTestId('coach-evidence-source')).toHaveLength(1);
  });

  it('drops a second reason whose every line of evidence the lead already shows', async () => {
    const source = { kind: 'provider' as const, providers: ['strava'] };
    const refused = { key: 'coachHome.evidence.sourceRefusedSince', params: { provider: 'strava', date: day(0) }, source, at: `${day(0)}T10:19:00Z` };
    const received = { key: 'coachHome.evidence.lastReceived', params: { provider: 'strava', date: day(-8) }, source, at: `${day(-8)}T08:19:00Z` };
    const dead = {
      ...JULIEN,
      attention: [
        { ...JULIEN.attention[0], evidence: [received, refused] },
        reason({
          id: 'provider_reconnect:strava',
          kind: 'provider_reconnect',
          horizon: 'today',
          rank: 31,
          key: 'coachHome.reason.providerReconnect',
          params: { provider: 'strava' },
          evidence: [refused, received],
        }),
      ],
    };
    const screen = renderAthlete('julien', [dead]);
    const why = within(await screen.findByTestId('coach-why'));
    expect(why.getByTestId('coach-reason-nothing_received')).toBeTruthy();
    expect(why.queryByTestId('coach-reason-provider_reconnect:strava')).toBeNull();
    expect(why.getAllByTestId('coach-evidence')).toHaveLength(2);
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

  it('marks today as a tick above and below the bands, never a line through their labels', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-plan-block'));
    const marker = block.getByTestId('coach-today-line', { hidden: true });
    expect(within(marker).getAllByTestId('coach-today-tick', { hidden: true })).toHaveLength(2);
  });

  it('names today\'s phase under the row, and draws a band\'s label only when the band can hold it', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-plan-block'));
    expect(block.getByTestId('coach-plan-note')).toHaveTextContent(/^Build · Week 3 of 12/);
    const build = block.getByTestId('coach-phase-build');
    const taper = block.getByTestId('coach-phase-taper');
    fireEvent(build, 'layout', { nativeEvent: { layout: { x: 0, y: 0, width: 240, height: 22 } } });
    fireEvent(taper, 'layout', { nativeEvent: { layout: { x: 243, y: 0, width: 24, height: 22 } } });
    measureLabel(block, 'build', 30);
    measureLabel(block, 'taper', 32);
    expect(within(build).getByTestId('coach-phase-label')).toHaveTextContent('Build');
    // Too short for « Taper »: no « T… », and the phase keeps its name for a screen reader.
    expect(within(taper).queryByTestId('coach-phase-label')).toBeNull();
    expect(taper.props.accessibilityLabel).toBe('Taper');
  });

  it('decides from the label as drawn in the coach\'s language, not from a width tuned to English', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-plan-block'));
    const taper = block.getByTestId('coach-phase-taper');
    // 60 points held « Taper » under the old fixed rule; a label that needs
    // 58 of them (the French « Affûtage ») does not fit inside the padding.
    fireEvent(taper, 'layout', { nativeEvent: { layout: { x: 0, y: 0, width: 60, height: 22 } } });
    measureLabel(block, 'taper', 58);
    expect(within(taper).queryByTestId('coach-phase-label')).toBeNull();
    // The same band holds a label that needs 40.
    measureLabel(block, 'taper', 40);
    expect(within(taper).getByTestId('coach-phase-label')).toHaveTextContent('Taper');
  });

  it('draws no band label until the label itself has been measured', async () => {
    const screen = renderAthlete('lea');
    const block = within(await screen.findByTestId('coach-plan-block'));
    const build = block.getByTestId('coach-phase-build');
    fireEvent(build, 'layout', { nativeEvent: { layout: { x: 0, y: 0, width: 240, height: 22 } } });
    expect(within(build).queryByTestId('coach-phase-label')).toBeNull();
  });

  it('draws a plan that ran out as a transition from its last planned day, not as its old phases', async () => {
    const plan = LEA.plan!;
    const ended = {
      ...LEA,
      attention: [],
      plan: {
        ...plan,
        phase: null,
        week: null,
        ends: day(-11),
        phases: [
          { kind: 'base' as const, start: day(-60), end: day(-30), weeks: 4 },
          { kind: 'build' as const, start: day(-30), end: day(-11), weeks: 3 },
        ],
      },
      season: { state: 'plan_ended' as const, race: null, ends: day(-11) },
    };
    const screen = renderAthlete('lea', [ended]);
    const block = within(await screen.findByTestId('coach-plan-block'));
    expect(block.getByTestId('coach-plan-ended')).toBeTruthy();
    expect(block.queryByTestId('coach-today-line', { hidden: true })).toBeNull();
    expect(block.queryByTestId('coach-phase-base')).toBeNull();
    expect(block.queryByText(/No A race/)).toBeNull();
    expect(block.getAllByText('Transition').length).toBeGreaterThan(0);
  });

  it('dates a cut reading by the last day a source delivered, not by the day it was computed', async () => {
    const recomputed = { ...JULIEN, training: { ...JULIEN.training!, form: { band: 'fresh' as const, as_of: day(0) } } };
    const screen = renderAthlete('julien', [recomputed]);
    const block = within(await screen.findByTestId('coach-form-block'));
    const lastDelivered = new Date(`${day(-6)}T12:00:00`).toLocaleDateString('en', { day: 'numeric', month: 'short' });
    expect(block.getByText(new RegExp(`Read on ${lastDelivered}`))).toBeTruthy();
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
  it('writes to the athlete in the room of the group the reason names, as the server finds or opens it', async () => {
    mockOpenGroupRoom.mockResolvedValue({ id: 'room-tri', group_id: 'g-tri' });
    const screen = renderAthlete('lea');
    fireEvent.press(await screen.findByTestId('coach-primary-action'));
    // The primary action is the group room, so the secondary one would repeat it.
    expect(screen.queryByTestId('coach-write-group')).toBeNull();
    await waitFor(() => expect(mockOpenGroupRoom).toHaveBeenCalledWith('g-tri'));
    await waitFor(() =>
      expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/chat/[conversationId]', params: { conversationId: 'room-tri' } }),
    );
    // Never the Chat list, which does not hold the group.
    expect(mockPush).not.toHaveBeenCalledWith('/(app)/(tabs)/(chat)');
  });

  it('labels the write action "Write in the group", the place it opens', async () => {
    const screen = renderAthlete('lea');
    expect(await screen.findByTestId('coach-primary-action')).toHaveTextContent('Write in the group');
    expect(screen.queryByText('Message them')).toBeNull();
  });

  it('says the room is opening and takes no second tap while it does', async () => {
    let resolve: (room: unknown) => void = () => undefined;
    mockOpenGroupRoom.mockReturnValue(new Promise((r) => (resolve = r)));
    const screen = renderAthlete('lea');
    const button = await screen.findByTestId('coach-primary-action');
    fireEvent.press(button);
    fireEvent.press(button);
    await waitFor(() => expect(screen.getByTestId('coach-primary-action')).toHaveTextContent('Opening the group…'));
    fireEvent.press(screen.getByTestId('coach-primary-action'));
    expect(mockOpenGroupRoom).toHaveBeenCalledTimes(1);
    await act(async () => resolve({ id: 'room-tri', group_id: 'g-tri' }));
    expect(mockPush).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('coach-primary-action')).toHaveTextContent('Write in the group');
  });

  it('says why a room could not be opened instead of dropping the coach on the Chat list', async () => {
    mockOpenGroupRoom.mockRejectedValue(new Error('refused'));
    const screen = renderAthlete('lea');
    fireEvent.press(await screen.findByTestId('coach-primary-action'));
    await waitFor(() =>
      expect(Alert.alert).toHaveBeenCalledWith('Error', "We couldn't open the group room. Please try again."),
    );
    expect(mockPush).not.toHaveBeenCalled();
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
    mockOpenGroupRoom.mockResolvedValue({ id: 'room-tri', group_id: 'g-tri' });
    fireEvent.press(screen.getByTestId('coach-write-group'));
    await waitFor(() =>
      expect(mockPush).toHaveBeenLastCalledWith({ pathname: '/(app)/chat/[conversationId]', params: { conversationId: 'room-tri' } }),
    );
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
