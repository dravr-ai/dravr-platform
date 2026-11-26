// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's athlete workspace on the phone — the tabs, the two weeks from the subject plan, the proposals, the editor, the check, the coach's call, the history (carnet#1818)
// ABOUTME: Pins which dates read « Proposé », what Accepter, Refuser, « C'est mon choix » and its withdrawal send, where the editor opens, and which thread each tab draws

import React from 'react';
import { Alert } from 'react-native';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type {
  CoachingGroup,
  CoachRosterAthlete,
  PlanChangeEvent,
  PlanDay,
  SubjectPlanHistory,
  SubjectPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';
import { ROSTER } from '../integration/app/helpers/coachRosterFixtures';
import { apiRefusal } from '../integration/app/helpers/apiRefusal';

const COACH_ID = 'user-coach';
const PLAN_ID = 'conv-plan-lea';
const ROOM_ID = 'conv-room-lea';
const GROUP = { id: 'g-lea', name: 'TRAKA' };
const LEA = { user_id: 'u-lea', display_name: 'Léa Gagnon' };

const mockPush = jest.fn();
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
    useLocalSearchParams: () => ({ athleteId: 'u-lea' }),
  }),
);
jest.mock('expo-router/react-navigation', () => ({
  ...jest.requireActual('expo-router/react-navigation'),
  useHeaderHeight: () => 0,
}));
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'user-coach', manages_roster: true }, isAuthenticated: true }),
}));

const mockApi = {
  getRoster: jest.fn(),
  getConversations: jest.fn(),
  getGroup: jest.fn(),
  getSubjectPlan: jest.fn(),
  getSubjectPlanHistory: jest.fn(),
  revertSubjectDraftDate: jest.fn(),
  setSubjectDraftDayCoachCall: jest.fn(),
  listParticipants: jest.fn(),
  openAthleteThread: jest.fn(),
  openGroupRoom: jest.fn(),
  getSubjectFamilies: jest.fn(),
  previewSubjectSession: jest.fn(),
};

// The network only: the workspace, its hooks, the draft controls and the day
// editor run for real over it.
jest.mock('../src/services/api', () => ({
  coachRosterApi: { getRoster: (...a: unknown[]) => mockApi.getRoster(...a) },
  chatApi: {
    getConversations: (...a: unknown[]) => mockApi.getConversations(...a),
    getSubjectPlan: (...a: unknown[]) => mockApi.getSubjectPlan(...a),
    getSubjectPlanHistory: (...a: unknown[]) => mockApi.getSubjectPlanHistory(...a),
    revertSubjectDraftDate: (...a: unknown[]) => mockApi.revertSubjectDraftDate(...a),
    setSubjectDraftDayCoachCall: (...a: unknown[]) => mockApi.setSubjectDraftDayCoachCall(...a),
    listParticipants: (...a: unknown[]) => mockApi.listParticipants(...a),
    openAthleteThread: (...a: unknown[]) => mockApi.openAthleteThread(...a),
    openGroupRoom: (...a: unknown[]) => mockApi.openGroupRoom(...a),
    getSubjectFamilies: (...a: unknown[]) => mockApi.getSubjectFamilies(...a),
    previewSubjectSession: (...a: unknown[]) => mockApi.previewSubjectSession(...a),
  },
  groupsApi: { getGroup: (...a: unknown[]) => mockApi.getGroup(...a) },
}));

// The thread inside the agent's and the group's tabs is the chat screen's
// own ChatThread, whose plan surface is pinned by the MessageList and
// ChatThread suites. Here a stand-in shows which conversation, which room and
// which flags the workspace hands it, and offers a plan card's way back.
jest.mock('../src/screens/coach-home/workspace/WorkspaceThread', () => {
  const React = require('react');
  const { Pressable, Text, View } = require('react-native');
  return {
    WorkspaceThread: (props: {
      conversationId: string;
      groupId: string | null;
      planSurface?: { kind: string; onShowCalendar: () => void } | null;
      pendingSend?: string | null;
      coachRoomName?: string | null;
    }) =>
      React.createElement(
        View,
        { testID: 'thread-stub' },
        React.createElement(Text, { testID: 'thread-stub-conversation' }, props.conversationId),
        React.createElement(Text, { testID: 'thread-stub-group' }, props.groupId ?? ''),
        React.createElement(Text, { testID: 'thread-stub-surface' }, props.planSurface?.kind ?? ''),
        React.createElement(Text, { testID: 'thread-stub-pending' }, props.pendingSend ?? ''),
        React.createElement(Text, { testID: 'thread-stub-room-name' }, props.coachRoomName ?? ''),
        props.planSurface
          ? React.createElement(Pressable, { testID: 'thread-stub-plan-card', onPress: props.planSurface.onShowCalendar })
          : null,
      ),
  };
});

import { AthleteWorkspaceScreen } from '../src/screens/coach-home/workspace/AthleteWorkspaceScreen';
import { threadHref } from '../src/navigation/routes';

function day(date: string, workout: string, extra: Partial<PlanDay> = {}): PlanDay {
  return { date, position: 0, sport: 'ride', workout, duration_min: 60, intensity: 'Z2', rest: false, ...extra };
}

const CONVENTION = { kind: 'convention' as const, label: 'Coaching convention' };

/** Five sessions on one date: the most a date holds, so it offers no « + Séance ». */
const FULL_DATE = [0, 1, 2, 3, 4].map((position) => day('2026-12-02', `Block ${position + 1}`, { position }));

/** The live fortnight, and the draft over it: Wednesday and Thursday swapped, a session added on Saturday. */
function plans(): { live: WorkoutPlan; draft: WorkoutPlan } {
  const weekOne = (swapped: boolean): PlanDay[] => [
    day('2026-11-23', 'Past long ride', { evidence: CONVENTION }),
    day('2026-11-24', 'CrossFit strength', { sport: 'crossfit' }),
    day('2026-11-25', swapped ? 'CrossFit moved' : 'Track endurance'),
    day('2026-11-26', swapped ? 'Track endurance' : 'CrossFit moved'),
    day('2026-11-28', 'Hike around Sutton', { sport: 'hike' }),
    ...(swapped ? [day('2026-11-28', 'Hip mobility', { position: 1, sport: 'mobility', added: true })] : []),
    day('2026-11-29', 'Long gravel ride', { evidence: CONVENTION }),
  ];
  const weekTwo: PlanDay[] = [
    ...FULL_DATE,
    day('2026-12-05', 'Kept hike', { coach_call: true, evidence: CONVENTION }),
    day('2026-12-06', 'Long ride with sprints', { outside: true }),
  ];
  const weeks = (swapped: boolean) => [
    { week_start: '2026-11-23', focus: 'Prep', current: true, days: weekOne(swapped) },
    { week_start: '2026-11-30', focus: 'Base', current: false, days: weekTwo },
  ];
  const base = {
    goal_race: { name: 'The Traka 200', date: '2027-04-30', discipline: 'gravel', priority: 'A' as const },
    phases: [],
    weeks_deferred: 20,
  };
  return { live: { ...base, weeks: weeks(false) }, draft: { ...base, weeks: weeks(true) } };
}

/** The dates the server names proposed in the draft `plans()` lays: the swap and the added session. */
const PROPOSED = ['2026-11-25', '2026-11-26', '2026-11-28'];

function subjectPlan(draft = true): SubjectPlanResponse {
  const { live, draft: drafted } = plans();
  return {
    athlete: LEA,
    today: '2026-11-25',
    live,
    draft: draft ? drafted : null,
    draft_state: draft ? { draft_id: 'draft-1', updated_at: '2026-11-25T10:05:00Z', conflicts: [], season_relaid: false } : null,
    proposed_dates: draft ? PROPOSED : [],
  };
}

/** The plan once « Refuser » put Wednesday back: the live entry on the date, which the server proposes no more. */
function withWednesdayRefused(): SubjectPlanResponse {
  const plan = subjectPlan();
  const draft = plan.draft as WorkoutPlan;
  const weeks = draft.weeks.map((week, index) =>
    index === 0
      ? { ...week, days: week.days.map((entry) => (entry.date === '2026-11-25' ? day('2026-11-25', 'Track endurance') : entry)) }
      : week,
  );
  return { ...plan, draft: { ...draft, weeks }, proposed_dates: PROPOSED.filter((date) => date !== '2026-11-25') };
}

/** The plan with `kept` set as the coach's call on the one session of each date named. */
function withCalls(calls: Record<string, boolean>): SubjectPlanResponse {
  const plan = subjectPlan();
  const draft = plan.draft as WorkoutPlan;
  const weeks = draft.weeks.map((week) => ({
    ...week,
    days: week.days.map((entry) => (entry.date in calls ? { ...entry, coach_call: calls[entry.date] } : entry)),
  }));
  return { ...plan, draft: { ...draft, weeks } };
}

const HISTORY: SubjectPlanHistory = {
  events: [
    { kind: 'agent_draft', source: 'agent', dates: ['2026-11-25', '2026-11-26'], created_at: '2026-11-02T10:05:00Z' },
    { kind: 'day_add', source: 'coach', dates: ['2026-11-28'], created_at: '2026-11-01T09:40:00Z' },
  ],
};

function group(solo: boolean): CoachingGroup {
  return {
    id: GROUP.id,
    tenant_id: 'tenant-a',
    name: GROUP.name,
    description: null,
    agent_id: 'agent-endurance',
    agent_title: 'Coach Max',
    agent_handle: 'max',
    owner_id: COACH_ID,
    coach_user_id: COACH_ID,
    coach_display_name: 'Phil',
    peer_data_sharing: false,
    respond_mode: 'all',
    digest_mode: 'off',
    max_members: 10,
    is_active: true,
    created_at: '2026-10-01T00:00:00Z',
    updated_at: '2026-10-01T00:00:00Z',
    ...(solo ? { solo_athlete: LEA } : {}),
  };
}

const planThread = {
  id: PLAN_ID,
  title: LEA.display_name,
  group_id: null,
  subject_user_id: LEA.user_id,
  subject_group_id: GROUP.id,
  subject_display_name: LEA.display_name,
  message_count: 2,
  created_at: '2026-11-14T00:00:00Z',
  updated_at: '2026-11-25T10:05:00Z',
};
const room = {
  id: ROOM_ID,
  title: GROUP.name,
  group_id: GROUP.id,
  message_count: 3,
  created_at: '2026-10-01T00:00:00Z',
  updated_at: '2026-11-25T10:04:00Z',
};

const ATHLETE: CoachRosterAthlete = { user_id: LEA.user_id, name: LEA.display_name, groups: [GROUP], status: 'shared', attention: [] };

function renderWorkspace(athlete: CoachRosterAthlete = ATHLETE) {
  mockApi.getRoster.mockResolvedValue({ ...ROSTER, athletes: [athlete] });
  // A mutation's default five-minute collection timer would keep the runner
  // alive after the last test, so mutations are collected at once. Queries
  // are never collected (Infinity schedules no timer), closer to the app's
  // minutes: a query collected the moment it has no observer can drop a page
  // still loading, and the history's next page twice failed to land on a
  // loaded CI worker with queries collected at once. Each test has its own
  // client, so no cache outlives it.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <AthleteWorkspaceScreen />
    </QueryClientProvider>,
  );
}

/** A date of the two weeks. */
const dateCell = (date: string) => screen.getByTestId(`workspace-date-${date}`);

/** Every date of the two weeks, in order. */
const dateIds = () => screen.getAllByTestId(/^workspace-date-\d/).map((cell) => String(cell.props.testID));

/**
 * The dates marked « Proposé ». The badge is hidden from a screen reader,
 * which hears the same in the date's heading, so it is found among hidden
 * elements.
 */
const proposedDates = () =>
  dateIds().filter(
    (id) => within(screen.getByTestId(id)).queryByTestId('workspace-date-proposed', { includeHiddenElements: true }) !== null,
  );

beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  mockApi.getConversations.mockResolvedValue({ conversations: [planThread, room], total: 2 });
  mockApi.getGroup.mockResolvedValue(group(true));
  mockApi.getSubjectPlan.mockResolvedValue(subjectPlan());
  mockApi.getSubjectPlanHistory.mockResolvedValue(HISTORY);
  mockApi.listParticipants.mockResolvedValue([
    { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-11-14T00:00:00Z' },
  ]);
  mockApi.revertSubjectDraftDate.mockResolvedValue(withWednesdayRefused());
  mockApi.setSubjectDraftDayCoachCall.mockResolvedValue(subjectPlan());
  // The editor's family catalogue never answers: these tests open the sheet, never save from it.
  mockApi.getSubjectFamilies.mockReturnValue(new Promise(() => undefined));
  mockApi.previewSubjectSession.mockResolvedValue({ derived: null, findings: [] });
  mockApi.openAthleteThread.mockResolvedValue(planThread);
  mockApi.openGroupRoom.mockResolvedValue(room);
});


/** The history paging test's budget: three waits of up to five seconds each. */
const HISTORY_PAGING_TIMEOUT_MS = 20_000;

describe('the tabs', () => {
  it('names the athlete in the header and opens on the plan, beside History, the agent and the group', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(screen.getByTestId('stack-header-title')).toHaveTextContent('Léa Gagnon');
    expect(screen.getByTestId('workspace-tabs-plan')).toHaveTextContent('Plan');
    expect(screen.getByTestId('workspace-tabs-plan')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('workspace-tabs-history')).toHaveTextContent('History');
    // The agent's tab goes by the name the athlete's room gives it.
    expect(screen.getByTestId('workspace-tabs-dravr')).toHaveTextContent('Coach Max');
    expect(screen.getByTestId('workspace-tabs-group')).toHaveTextContent('Group');
  });

  it("draws the coach's plan thread in the agent's tab, its plan cards leading back to the plan", async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-tabs-dravr'));
    expect(screen.getByTestId('thread-stub-conversation')).toHaveTextContent(PLAN_ID);
    expect(screen.getByTestId('thread-stub-group')).toHaveTextContent('');
    expect(screen.getByTestId('thread-stub-surface')).toHaveTextContent('workspace');
    expect(within(screen.getByTestId('workspace-dravr')).getByText('Only you and Coach Max see what follows')).toBeTruthy();

    fireEvent.press(screen.getByTestId('thread-stub-plan-card'));
    expect(screen.getByTestId('workspace-panel-plan')).toBeTruthy();
    expect(screen.getByTestId('workspace-calendar')).toBeTruthy();
  });

  it("draws the athlete's room in the group's tab, with no plan surface, under the room's name", async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-tabs-group'));
    expect(screen.getByTestId('thread-stub-conversation')).toHaveTextContent(ROOM_ID);
    expect(screen.getByTestId('thread-stub-group')).toHaveTextContent('g-lea');
    expect(screen.getByTestId('thread-stub-surface')).toHaveTextContent('');
    expect(screen.getByTestId('thread-stub-room-name')).toHaveTextContent('TRAKA');
  });

  it('offers to open the room when the coach holds no conversation in it yet, and draws it once opened', async () => {
    mockApi.getConversations.mockResolvedValue({ conversations: [planThread], total: 1 });
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-tabs-group'));
    expect(screen.getByTestId('workspace-room-unopened')).toHaveTextContent(
      'The TRAKA room: every member and the agent read what you write here.Write in the group',
    );
    expect(screen.queryByTestId('thread-stub')).toBeNull();

    fireEvent.press(screen.getByTestId('workspace-open-room'));
    await waitFor(() => expect(screen.getByTestId('thread-stub-conversation')).toHaveTextContent(ROOM_ID));
    expect(mockApi.openGroupRoom).toHaveBeenCalledWith('g-lea');
  });

  it('says an athlete whose groups hold other athletes has no room, and offers their group instead', async () => {
    mockApi.getGroup.mockResolvedValue(group(false));
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-tabs-group'));
    expect(screen.getByTestId('workspace-no-room')).toHaveTextContent(
      'Léa Gagnon has no room of their own with you: their groups include other athletes.Write in the group',
    );
    fireEvent.press(screen.getByTestId('workspace-write-group'));
    await waitFor(() => expect(mockPush).toHaveBeenCalledWith(threadHref(ROOM_ID)));
    expect(mockApi.openGroupRoom).toHaveBeenCalledWith('g-lea');
  });
});

describe('the plan', () => {
  it('draws the two weeks from the draft and marks only the dates the server names proposed', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(dateIds()).toHaveLength(14);
    expect(within(dateCell('2026-11-25')).getByText('CrossFit moved')).toBeTruthy();
    expect(within(dateCell('2026-11-28')).getByText('Hip mobility')).toBeTruthy();
    expect(proposedDates()).toEqual(['workspace-date-2026-11-25', 'workspace-date-2026-11-26', 'workspace-date-2026-11-28']);
    expect(
      within(dateCell('2026-11-25')).getByTestId('workspace-date-proposed', { includeHiddenElements: true }),
    ).toHaveTextContent('Proposed');
    // A screen reader hears the proposal with the date's heading.
    expect(within(dateCell('2026-11-25')).getByRole('header')).toHaveProp('accessibilityLabel', 'Wednesday, November 25, Proposed');
    expect(within(dateCell('2026-11-24')).getByRole('header')).toHaveProp('accessibilityLabel', 'Tuesday, November 24');
    // A date is a group, never a box: a proposed one sits on the faint
    // primary wash, every other on the paper (DESIGN.md "Phone", "A wash is not an alert").
    expect(dateCell('2026-11-25').props.className).toContain('bg-primary/5');
    expect(dateCell('2026-11-24').props.className).not.toContain('bg-primary/5');
    for (const id of dateIds()) {
      expect(screen.getByTestId(id).props.className).not.toMatch(/\bborder\b|\brounded/);
    }
    expect(screen.getByTestId('workspace-accept-2026-11-25')).toHaveTextContent('Accept');
    expect(screen.getByTestId('workspace-refuse-2026-11-25')).toHaveTextContent('Decline');
    expect(screen.queryByTestId('workspace-accept-2026-11-24')).toBeNull();
    expect(screen.queryByTestId('workspace-refuse-2026-11-24')).toBeNull();
    // The draft's own publish controls, as every plan card carries them.
    expect(await screen.findByTestId('plan-draft-publish')).toHaveTextContent('Publish to Léa Gagnon');
  });

  it('puts the live plan back on a refused date, through the date revert, and proposes it no more', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');
    mockApi.getSubjectPlan.mockResolvedValue(withWednesdayRefused());

    fireEvent.press(screen.getByTestId('workspace-refuse-2026-11-25'));

    await waitFor(() => expect(mockApi.revertSubjectDraftDate).toHaveBeenCalledWith(PLAN_ID, '2026-11-25', 'en'));
    await waitFor(() => expect(screen.queryByTestId('workspace-refuse-2026-11-25')).toBeNull());
    expect(within(dateCell('2026-11-25')).getByText('Track endurance')).toBeTruthy();
    expect(proposedDates()).toEqual(['workspace-date-2026-11-26', 'workspace-date-2026-11-28']);
  });

  it('says why a refusal was not written, and keeps the date proposed', async () => {
    mockApi.revertSubjectDraftDate.mockRejectedValue(apiRefusal(500, { message: 'database unavailable' }));
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-refuse-2026-11-25'));

    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('This change could not be declined.', expect.any(String)));
    expect(screen.getByTestId('workspace-refuse-2026-11-25')).toBeTruthy();
  });

  it('holds an acceptance for this visit and writes nothing', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-accept-2026-11-26'));

    const acceptedMark = within(dateCell('2026-11-26')).getByTestId('workspace-date-accepted', { includeHiddenElements: true });
    expect(within(acceptedMark).getByText('Accepted', { includeHiddenElements: true })).toBeTruthy();
    expect(within(dateCell('2026-11-26')).getByRole('header')).toHaveProp('accessibilityLabel', 'Thursday, November 26, Accepted');
    expect(screen.queryByTestId('workspace-accept-2026-11-26')).toBeNull();
    expect(screen.getByTestId('workspace-refuse-2026-11-26')).toBeTruthy();
    expect(mockApi.revertSubjectDraftDate).not.toHaveBeenCalled();
    expect(mockApi.setSubjectDraftDayCoachCall).not.toHaveBeenCalled();
  });

  it('keeps an acceptance while the coach reads another tab and comes back to the plan', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');
    fireEvent.press(screen.getByTestId('workspace-accept-2026-11-26'));

    fireEvent.press(screen.getByTestId('workspace-tabs-history'));
    await screen.findAllByTestId('workspace-history-event');
    expect(screen.queryByTestId('workspace-calendar')).toBeNull();
    fireEvent.press(screen.getByTestId('workspace-tabs-plan'));

    await screen.findByTestId('workspace-calendar');
    expect(within(dateCell('2026-11-26')).getByRole('header')).toHaveProp('accessibilityLabel', 'Thursday, November 26, Accepted');
    expect(screen.queryByTestId('workspace-accept-2026-11-26')).toBeNull();
    // Only the date accepted: the others are still proposed.
    expect(screen.getByTestId('workspace-accept-2026-11-25')).toBeTruthy();
  });

  it('opens the editor on the session pressed', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-session-2026-11-28-1'));

    expect(await screen.findByTestId('session-editor-title')).toHaveTextContent('Edit Sat, Nov 28');
    expect(screen.getByTestId('session-editor-workout')).toHaveProp('value', 'Hip mobility');
  });

  it('opens the editor on a blank session after a date’s sessions, and holds « + Séance » on a full date', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(screen.getByTestId('workspace-add-2026-12-02')).toHaveProp('accessibilityState', { disabled: true });
    expect(within(screen.getByTestId('workspace-add-2026-11-28')).getByText('Session')).toBeTruthy();
    expect(screen.getByTestId('workspace-add-2026-11-28')).toHaveProp('accessibilityLabel', 'Add a session on Sat, Nov 28');
    fireEvent.press(screen.getByTestId('workspace-add-2026-11-27'));

    expect(await screen.findByTestId('session-editor-title')).toHaveTextContent('Add a session on Fri, Nov 27');
    expect(screen.getByTestId('session-editor-workout')).toHaveProp('value', '');
  });

  it('asks before publishing about each session from today on that rests on convention or outside its evidence', async () => {
    renderWorkspace();
    const check = within(await screen.findByTestId('workspace-prepublish'));

    // A heading, so a screen reader moving heading by heading reaches the check as it reaches each date.
    expect(check.getByRole('header', { name: 'Before you publish' })).toBeTruthy();
    // What keeping a session means is said once, under the section's title.
    expect(check.getByText('You keep this session as it is: Dravr will describe it as your call, without arguing it.')).toBeTruthy();
    expect(screen.getByTestId('workspace-prepublish').props.className).not.toMatch(/\bborder\b|\brounded/);
    // The past date and the session already kept as the coach's call are not asked about.
    expect(screen.queryByTestId('workspace-check-2026-11-23-0')).toBeNull();
    expect(screen.queryByTestId('workspace-check-2026-12-05-0')).toBeNull();
    expect(screen.getByTestId('workspace-check-2026-11-29-0')).toHaveTextContent(
      "Long gravel ride, Sun, Nov 29: follows a training convention rather than Léa Gagnon's data.It's my callReview",
    );
    expect(screen.getByTestId('workspace-check-2026-12-06-0')).toHaveTextContent(
      "Long ride with sprints, Sun, Dec 6: sits outside the evidence for its workout family.It's my callReview",
    );
  });

  it("keeps a session as the coach's call from the check, for that session as it is shown", async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-prepublish');
    mockApi.setSubjectDraftDayCoachCall.mockResolvedValue(withCalls({ '2026-11-29': true }));
    mockApi.getSubjectPlan.mockResolvedValue(withCalls({ '2026-11-29': true }));

    fireEvent.press(screen.getByTestId('workspace-check-keep-2026-11-29-0'));

    await waitFor(() =>
      expect(mockApi.setSubjectDraftDayCoachCall).toHaveBeenCalledWith(
        PLAN_ID,
        '2026-11-29',
        0,
        true,
        { sport: 'ride', workout: 'Long gravel ride' },
        'en',
      ),
    );
    await waitFor(() => expect(screen.queryByTestId('workspace-check-2026-11-29-0')).toBeNull());
    expect(within(dateCell('2026-11-29')).getByTestId('plan-day-coach-call')).toHaveTextContent("Coach's call");
  });

  it('opens the editor on the session the check names from « Revoir »', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-prepublish');

    fireEvent.press(screen.getByTestId('workspace-check-review-2026-12-06-0'));

    expect(await screen.findByTestId('session-editor-title')).toHaveTextContent('Edit Sun, Dec 6');
    expect(screen.getByTestId('session-editor-workout')).toHaveProp('value', 'Long ride with sprints');
  });

  it("marks a session kept as the coach's call and withdraws the call from its undo", async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');
    const kept = within(dateCell('2026-12-05'));
    expect(kept.getByTestId('plan-day-coach-call')).toHaveTextContent("Coach's call");
    expect(screen.queryByTestId('workspace-call-undo-2026-11-29-0')).toBeNull();
    mockApi.setSubjectDraftDayCoachCall.mockResolvedValue(withCalls({ '2026-12-05': false }));
    mockApi.getSubjectPlan.mockResolvedValue(withCalls({ '2026-12-05': false }));

    expect(kept.getByTestId('workspace-call-undo-2026-12-05-0')).toHaveTextContent('Withdraw my call');
    fireEvent.press(kept.getByTestId('workspace-call-undo-2026-12-05-0'));

    await waitFor(() =>
      expect(mockApi.setSubjectDraftDayCoachCall).toHaveBeenCalledWith(
        PLAN_ID,
        '2026-12-05',
        0,
        false,
        { sport: 'ride', workout: 'Kept hike' },
        'en',
      ),
    );
    // Withdrawn, the session is asked about again before publish.
    expect(await screen.findByTestId('workspace-check-2026-12-05-0')).toBeTruthy();
    expect(within(dateCell('2026-12-05')).queryByTestId('plan-day-coach-call')).toBeNull();
  });

  it('says an athlete who stopped sharing can no longer have their plan changed', async () => {
    mockApi.getSubjectPlan.mockRejectedValue(apiRefusal(403, { error_code: 'coach_sharing_revoked' }));
    renderWorkspace();

    expect(await screen.findByTestId('workspace-consent-revoked')).toHaveTextContent(
      "Léa Gagnon stopped sharing with you, so their plan can't be changed.",
    );
    expect(screen.queryByTestId('workspace-calendar')).toBeNull();
  });

  it("says an athlete has no plan yet, and hands the agent's tab the plan request", async () => {
    mockApi.getSubjectPlan.mockResolvedValue({ ...subjectPlan(false), live: null });
    renderWorkspace();

    expect(await screen.findByTestId('workspace-no-plan')).toHaveTextContent('Léa Gagnon has no plan yet.Draft Léa Gagnon’s plan');
    fireEvent.press(screen.getByTestId('workspace-draft-plan'));

    expect(screen.getByTestId('workspace-tabs-dravr')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('thread-stub-pending')).toHaveTextContent(
      'Draft a training plan for Léa Gagnon, built from their recent training and their goals.',
      { exact: false },
    );
  });
});

describe('the history', () => {
  it('lists each change as « when · who · what », the agent by its name and the coach as you', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-tabs-history'));
    const lines = await screen.findAllByTestId('workspace-history-event');

    expect(screen.getByTestId('workspace-history-tab')).toHaveTextContent('What changed in the plan, newest first.', { exact: false });
    expect(lines).toHaveLength(2);
    expect(lines[0]).toHaveTextContent('Nov 2Coach MaxDraft saved · Wed, Nov 25 and Thu, Nov 26');
    expect(lines[1]).toHaveTextContent('Nov 1YouSession added · Sat, Nov 28');
    expect(screen.queryByTestId('workspace-history-more')).toBeNull();
    expect(mockApi.getSubjectPlanHistory).toHaveBeenCalledWith(PLAN_ID, { limit: 50 });
  });

  // A full page and the next: 101 lines rendered with the locale's date
  // formatters, which a loaded CI worker can take past jest's default five
  // seconds for the whole test (three waits of up to five each below).
  it('reads the older changes a page at a time, before the oldest shown, until a page comes back short', async () => {
    const full: PlanChangeEvent[] = Array.from({ length: 50 }, (_, index) => ({
      kind: 'day_edit',
      source: 'coach',
      dates: ['2026-11-25'],
      created_at: `2026-11-0${index < 25 ? 3 : 2}T${String(10 + (index % 12)).padStart(2, '0')}:${String(index).padStart(2, '0')}:00Z`,
    }));
    const oldest = full[full.length - 1].created_at;
    const older: SubjectPlanHistory = {
      events: [{ kind: 'publish', source: 'coach', dates: ['2026-11-25'], created_at: '2026-10-30T08:00:00Z' }],
    };
    mockApi.getSubjectPlanHistory.mockResolvedValueOnce({ events: full }).mockResolvedValueOnce(older);
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    // Fifty lines, each worded with the locale's date formatters: a render
    // that large can outlast the default second on a loaded worker, so every
    // wait below allows five.
    fireEvent.press(screen.getByTestId('workspace-tabs-history'));
    await waitFor(() => expect(screen.getAllByTestId('workspace-history-event')).toHaveLength(50), { timeout: 5000 });

    await act(async () => {
      fireEvent.press(screen.getByTestId('workspace-history-more'));
    });

    await waitFor(
      () => expect(mockApi.getSubjectPlanHistory).toHaveBeenLastCalledWith(PLAN_ID, { before: oldest, limit: 50 }),
      { timeout: 5000 },
    );
    await waitFor(() => expect(screen.getAllByTestId('workspace-history-event')).toHaveLength(51), { timeout: 5000 });
    expect(screen.getAllByTestId('workspace-history-event')[50]).toHaveTextContent('Oct 30YouPlan published · Wed, Nov 25');
    expect(screen.queryByTestId('workspace-history-more')).toBeNull();
  }, HISTORY_PAGING_TIMEOUT_MS);

  it('says when nothing is recorded yet', async () => {
    mockApi.getSubjectPlanHistory.mockResolvedValue({ events: [] });
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    fireEvent.press(screen.getByTestId('workspace-tabs-history'));
    expect(await screen.findByTestId('workspace-history-empty')).toHaveTextContent('No change is recorded yet.');
  });
});
