// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's athlete workspace — the two weeks from the subject plan, the proposals, the editor, the pre-publish check and the panel's tabs (carnet#1816)
// ABOUTME: Pins which dates read « Proposé », what Accepter, Refuser and « C'est mon choix » send, where the editor opens, and which thread each tab draws

import type { ReactNode } from 'react';
import { describe, it, expect, vi, beforeAll, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type {
  CoachingGroup,
  CoachRosterAthlete,
  PlanDay,
  SubjectPlanHistory,
  SubjectPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';
import { ToastProvider } from '../../ui';
import AthleteWorkspace from '../workspace/AthleteWorkspace';
import type { CoachDestinations } from '../useCoachDestinations';

const COACH_ID = 'user-coach';
const PLAN_ID = 'conv-plan-lea';
const ROOM_ID = 'conv-room-lea';
const GROUP = { id: 'g-lea', name: 'TRAKA' };
const LEA = { user_id: 'u-lea', display_name: 'Léa Gagnon' };

const api = vi.hoisted(() => ({
  getConversations: vi.fn(),
  getSubjectPlan: vi.fn(),
  getSubjectPlanHistory: vi.fn(),
  revertSubjectDraftDate: vi.fn(),
  setSubjectDraftDayCoachCall: vi.fn(),
  listParticipants: vi.fn(),
  openAthleteThread: vi.fn(),
  openGroupRoom: vi.fn(),
  publishSubjectPlan: vi.fn(),
  discardSubjectPlanDraft: vi.fn(),
  getSubjectFamilies: vi.fn(),
  previewSubjectSession: vi.fn(),
  getGroup: vi.fn(),
}));

// The network only: the workspace, its hooks, the draft controls and the day
// editor run for real over it. The server half of every write is pinned by
// the subject-plan route tests; the browser flow by the design sweep's mocks.
vi.mock('../../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => api.getConversations(...a),
    getSubjectPlan: (...a: unknown[]) => api.getSubjectPlan(...a),
    getSubjectPlanHistory: (...a: unknown[]) => api.getSubjectPlanHistory(...a),
    revertSubjectDraftDate: (...a: unknown[]) => api.revertSubjectDraftDate(...a),
    setSubjectDraftDayCoachCall: (...a: unknown[]) => api.setSubjectDraftDayCoachCall(...a),
    listParticipants: (...a: unknown[]) => api.listParticipants(...a),
    openAthleteThread: (...a: unknown[]) => api.openAthleteThread(...a),
    openGroupRoom: (...a: unknown[]) => api.openGroupRoom(...a),
    publishSubjectPlan: (...a: unknown[]) => api.publishSubjectPlan(...a),
    discardSubjectPlanDraft: (...a: unknown[]) => api.discardSubjectPlanDraft(...a),
    getSubjectFamilies: (...a: unknown[]) => api.getSubjectFamilies(...a),
    previewSubjectSession: (...a: unknown[]) => api.previewSubjectSession(...a),
  },
  groupsApi: { getGroup: (...a: unknown[]) => api.getGroup(...a) },
}));

vi.mock('../../../hooks/useAuth', () => ({ useAuth: () => ({ user: { id: COACH_ID } }) }));

const screenSize = vi.hoisted(() => ({ desktop: true }));
vi.mock('../../../hooks/useBreakpoint', () => ({
  useIsMobile: () => !screenSize.desktop,
  useIsTablet: () => false,
  useIsDesktop: () => screenSize.desktop,
}));

// The thread inside each tab is the chat surface's own, pinned by the
// ChatThread suite with the same two flags (plan cards as one line, no
// private lane). Here a stand-in shows which conversation and which flags the
// workspace hands it, and offers the plan card's way back to the calendar.
vi.mock('../../chat/EmbeddedChat', () => ({
  default: (props: {
    conversationId: string | null;
    privateLane?: boolean;
    planSurface?: { kind: string; onShowCalendar: () => void } | null;
    emptyState: ReactNode;
  }) => (
    <div
      data-testid="embedded-chat-stub"
      data-conversation={props.conversationId ?? ''}
      data-private-lane={String(props.privateLane ?? true)}
      data-plan-surface={props.planSurface?.kind ?? ''}
    >
      {props.conversationId === null ? props.emptyState : null}
      {props.planSurface ? (
        <button type="button" onClick={props.planSurface.onShowCalendar}>
          stand-in plan card
        </button>
      ) : null}
    </div>
  ),
}));

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
    phases: [
      { kind: 'prep' as const, start: '2026-11-16', end: '2026-11-30', weeks: 2, purpose: '', intent: '', current: true },
      { kind: 'base' as const, start: '2026-11-30', end: '2027-02-22', weeks: 12, purpose: '', intent: '', current: false },
    ],
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

/** The plan once « Refuser » put Wednesday back: the live entries on the date, which the server proposes no more. */
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

/** `plan` once the agent proposed Thursday again, with another session on it. */
function withThursdayReproposed(plan: SubjectPlanResponse): SubjectPlanResponse {
  const draft = plan.draft as WorkoutPlan;
  const weeks = draft.weeks.map((week, index) =>
    index === 0
      ? { ...week, days: week.days.map((entry) => (entry.date === '2026-11-26' ? day('2026-11-26', 'Track tempo') : entry)) }
      : week,
  );
  return { ...plan, draft: { ...draft, weeks } };
}

/** The plan once the coach kept each of `dates` as their call. */
function withCalls(dates: string[]): SubjectPlanResponse {
  const plan = subjectPlan();
  const draft = plan.draft as WorkoutPlan;
  const weeks = draft.weeks.map((week) => ({
    ...week,
    days: week.days.map((entry) => (dates.includes(entry.date) ? { ...entry, coach_call: true } : entry)),
  }));
  return { ...plan, draft: { ...draft, weeks } };
}

const HISTORY: SubjectPlanHistory = {
  events: [
    { kind: 'agent_draft', source: 'agent', dates: ['2026-11-25', '2026-11-26'], created_at: '2026-11-25T10:05:00Z' },
    { kind: 'day_add', source: 'coach', dates: ['2026-11-28'], created_at: '2026-11-25T09:40:00Z' },
  ],
};

function group(solo: boolean): CoachingGroup {
  return {
    id: GROUP.id,
    tenant_id: 'tenant-a',
    name: GROUP.name,
    description: null,
    agent_id: 'agent-endurance',
    agent_title: 'Dravr',
    agent_handle: 'dravr',
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
const room = { id: ROOM_ID, title: GROUP.name, group_id: GROUP.id, message_count: 3, created_at: '2026-10-01T00:00:00Z', updated_at: '2026-11-25T10:04:00Z' };

const ATHLETE: CoachRosterAthlete = { user_id: LEA.user_id, name: LEA.display_name, groups: [GROUP], status: 'shared', attention: [] };

function destinations(): CoachDestinations {
  return {
    writeToGroup: vi.fn(),
    perform: vi.fn(),
    openPlanThread: vi.fn(),
    openWorkspace: vi.fn(),
    pendingAthleteId: null,
    pendingRoomAthleteId: null,
    primaryIsGroupRoom: () => false,
  };
}

function renderWorkspace(athlete: CoachRosterAthlete = ATHLETE) {
  const navigate = vi.fn();
  const routes = destinations();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <AthleteWorkspace athlete={athlete} onNavigate={navigate} destinations={routes} />
      </ToastProvider>
    </QueryClientProvider>,
  );
  return { navigate, routes };
}

/** A date's cell on the calendar. */
const dateCell = (date: string) => screen.getByTestId(`workspace-date-${date}`);

/** Every date cell, its chips aside. */
const dateCells = () => screen.getAllByTestId(/^workspace-date-\d/);

/** A date's heading, which says where its proposal stands. */
const dateHeading = (date: string) => within(dateCell(date)).getByRole('heading', { level: 3 });

describe('the athlete workspace (carnet#1816)', () => {
  // The editor loads lazily on first open; loaded once here so the first test
  // to open it waits on the sheet, not on a cold module graph.
  beforeAll(async () => {
    await import('../../chat/SessionEditorSheet');
  });

  beforeEach(() => {
    vi.clearAllMocks();
    screenSize.desktop = true;
    api.getConversations.mockResolvedValue({ conversations: [planThread, room], total: 2 });
    api.getGroup.mockResolvedValue(group(true));
    api.getSubjectPlan.mockResolvedValue(subjectPlan());
    api.getSubjectPlanHistory.mockResolvedValue(HISTORY);
    api.listParticipants.mockResolvedValue([{ user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-11-14T00:00:00Z' }]);
    api.revertSubjectDraftDate.mockResolvedValue(withWednesdayRefused());
    api.setSubjectDraftDayCoachCall.mockResolvedValue(subjectPlan());
    api.getSubjectFamilies.mockResolvedValue({ families: [] });
    api.previewSubjectSession.mockResolvedValue({ derived: null, findings: [] });
    api.openAthleteThread.mockResolvedValue(planThread);
    api.openGroupRoom.mockResolvedValue(room);
  });

  it('draws the two weeks from the draft, marks only the dates it changes, and counts them on publish', async () => {
    renderWorkspace();

    await screen.findByTestId('workspace-calendar');
    expect(dateCells()).toHaveLength(14);
    expect(within(dateCell('2026-11-25')).getByText('CrossFit moved')).toBeInTheDocument();
    expect(within(dateCell('2026-11-28')).getByText('Hip mobility')).toBeInTheDocument();
    const proposed = dateCells().filter((cell) => cell.dataset.proposed === 'true');
    expect(proposed.map((cell) => cell.dataset.testid)).toEqual([
      'workspace-date-2026-11-25',
      'workspace-date-2026-11-26',
      'workspace-date-2026-11-28',
    ]);
    expect(within(dateCell('2026-11-25')).getByText(i18n.t('plan.workspace.proposed'))).toBeInTheDocument();
    expect(dateCell('2026-11-25')).toHaveAttribute('aria-current', 'date');
    expect(await screen.findByTestId('plan-draft-publish')).toHaveTextContent(i18n.t('plan.workspace.publishN', { count: 3 }));
    expect(screen.getByTestId('plan-draft-publish')).toBeEnabled();
    expect(screen.getByTestId('workspace-goal')).toHaveTextContent('The Traka 200');
  });

  it('marks the dates the server names proposed and no other, whatever the entries hold', async () => {
    api.getSubjectPlan.mockResolvedValue({ ...subjectPlan(), proposed_dates: ['2026-12-01'] });
    renderWorkspace();

    await screen.findByTestId('workspace-calendar');
    const proposed = dateCells().filter((cell) => cell.dataset.proposed === 'true');
    expect(proposed.map((cell) => cell.dataset.testid)).toEqual(['workspace-date-2026-12-01']);
    expect(await screen.findByTestId('plan-draft-publish')).toHaveTextContent(i18n.t('plan.workspace.publishOne', { count: 1 }));
  });

  it('marks only a convention or outside-evidence session, and a coach’s call in its own words', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(within(dateCell('2026-11-29')).getByTestId('workspace-session-evidence')).toHaveTextContent(
      i18n.t('messaging.evidence.convention'),
    );
    expect(within(dateCell('2026-12-06')).getByTestId('workspace-session-evidence')).toHaveTextContent(
      i18n.t('messaging.evidence.outside'),
    );
    expect(within(dateCell('2026-12-05')).getByTestId('workspace-session-coach-call')).toHaveTextContent(
      i18n.t('humanCoach.coachCall.badge'),
    );
    expect(within(dateCell('2026-12-05')).queryByTestId('workspace-session-evidence')).not.toBeInTheDocument();
    expect(within(dateCell('2026-11-24')).queryByTestId('workspace-session-evidence')).not.toBeInTheDocument();
  });

  it('opens the editor on a session from its block, and on a blank one from « + Séance »', async () => {
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    await user.click(screen.getByTestId('workspace-session-2026-11-24-0'));
    expect(await screen.findByRole('dialog', { name: i18n.t('plan.editor.editTitle', { date: 'Tue, Nov 24' }) })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: i18n.t('chat.close') }));

    await user.click(screen.getByTestId('workspace-add-2026-11-27'));
    expect(await screen.findByRole('dialog', { name: i18n.t('plan.editor.addTitle', { date: 'Fri, Nov 27' }) })).toBeInTheDocument();
  });

  it('names each session block for the session and its date, and holds back « + Séance » on a full date', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(screen.getByTestId('workspace-session-2026-11-24-0')).toHaveAccessibleName(
      i18n.t('plan.workspace.editSession', { session: 'CrossFit strength', date: 'Tue, Nov 24' }),
    );
    expect(screen.getByTestId('workspace-add-2026-12-02')).toBeDisabled();
    expect(screen.getByTestId('workspace-add-2026-12-01')).toBeEnabled();
  });

  it('refuses a proposed date by putting the live plan back on it', async () => {
    // The server answers the plan with the live entries back, and reads it so from then on.
    api.revertSubjectDraftDate.mockImplementation(async () => {
      api.getSubjectPlan.mockResolvedValue(withWednesdayRefused());
      return withWednesdayRefused();
    });
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    await user.click(screen.getByTestId('workspace-refuse-2026-11-25'));
    await waitFor(() => expect(api.revertSubjectDraftDate).toHaveBeenCalledWith(PLAN_ID, '2026-11-25', 'en'));
    await waitFor(() => expect(dateCell('2026-11-25')).not.toHaveAttribute('data-proposed'));
    expect(within(dateCell('2026-11-25')).getByText('Track endurance')).toBeInTheDocument();
  });

  it('accepts a proposal for this session only, sending nothing', async () => {
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    await user.click(screen.getByTestId('workspace-accept-2026-11-26'));
    expect(within(dateCell('2026-11-26')).getByTestId('workspace-date-accepted')).toHaveTextContent(
      i18n.t('plan.workspace.accepted'),
    );
    expect(screen.queryByTestId('workspace-accept-2026-11-26')).not.toBeInTheDocument();
    expect(screen.getByTestId('workspace-refuse-2026-11-26')).toBeInTheDocument();
    expect(api.revertSubjectDraftDate).not.toHaveBeenCalled();
    expect(api.setSubjectDraftDayCoachCall).not.toHaveBeenCalled();
  });

  it('asks again on a date accepted this session once the draft proposes other sessions on it', async () => {
    // Refusing Wednesday answers a plan in which the agent has since put
    // another session on Thursday.
    const answered = withThursdayReproposed(withWednesdayRefused());
    api.revertSubjectDraftDate.mockImplementation(async () => {
      api.getSubjectPlan.mockResolvedValue(answered);
      return answered;
    });
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    await user.click(screen.getByTestId('workspace-accept-2026-11-26'));
    expect(within(dateCell('2026-11-26')).getByTestId('workspace-date-accepted')).toBeInTheDocument();

    await user.click(screen.getByTestId('workspace-refuse-2026-11-25'));
    await waitFor(() => expect(within(dateCell('2026-11-26')).getByText('Track tempo')).toBeInTheDocument());
    expect(within(dateCell('2026-11-26')).queryByTestId('workspace-date-accepted')).not.toBeInTheDocument();
    expect(screen.getByTestId('workspace-accept-2026-11-26')).toBeInTheDocument();
  });

  it('says each date’s proposal in its heading, and leaves focus on the date once the coach answers', async () => {
    api.revertSubjectDraftDate.mockImplementation(async () => {
      api.getSubjectPlan.mockResolvedValue(withWednesdayRefused());
      return withWednesdayRefused();
    });
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(dateHeading('2026-11-26')).toHaveAccessibleName(`Thursday, November 26, ${i18n.t('plan.workspace.proposed')}`);
    expect(dateHeading('2026-11-27')).toHaveAccessibleName('Friday, November 27');

    // « Accepter » goes away once pressed: the date's heading takes focus and says it.
    await user.click(screen.getByTestId('workspace-accept-2026-11-26'));
    expect(dateHeading('2026-11-26')).toHaveFocus();
    expect(dateHeading('2026-11-26')).toHaveAccessibleName(`Thursday, November 26, ${i18n.t('plan.workspace.accepted')}`);

    // Both answers go once the date is put back: focus stays on the date.
    await user.click(screen.getByTestId('workspace-refuse-2026-11-25'));
    await waitFor(() => expect(dateHeading('2026-11-25')).toHaveAccessibleName('Wednesday, November 25'));
    expect(dateHeading('2026-11-25')).toHaveFocus();
  });

  it('leaves the coach on « Refuser » and says why when the date cannot be put back', async () => {
    api.revertSubjectDraftDate.mockRejectedValue(new Error('network down'));
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    const refuse = screen.getByTestId('workspace-refuse-2026-11-25');
    await user.click(refuse);
    expect(await screen.findByText(i18n.t('plan.workspace.refuseFailed'))).toBeInTheDocument();
    expect(refuse).toHaveFocus();
    expect(refuse).not.toHaveAttribute('aria-disabled');
    expect(dateCell('2026-11-25')).toHaveAttribute('data-proposed', 'true');
  });

  it('describes a session block by its length, the editor’s mark and where its evidence sits', async () => {
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(screen.getByTestId('workspace-session-2026-11-28-1')).toHaveAccessibleDescription(
      new RegExp(`Z2 ${i18n.t('plan.editor.added')}$`),
    );
    expect(screen.getByTestId('workspace-session-2026-11-29-0')).toHaveAccessibleDescription(
      new RegExp(`Z2 ${i18n.t('messaging.evidence.convention')}$`),
    );
  });

  it('asks before publishing about convention and outside sessions from today on that are not the coach’s call', async () => {
    const user = userEvent.setup();
    renderWorkspace();

    const check = await screen.findByTestId('workspace-prepublish');
    expect(within(check).getAllByRole('listitem').map((item) => item.dataset.testid)).toEqual([
      'workspace-check-2026-11-29-0',
      'workspace-check-2026-12-06-0',
    ]);

    await user.click(within(check).getByTestId('workspace-check-keep-2026-11-29-0'));
    await waitFor(() =>
      expect(api.setSubjectDraftDayCoachCall).toHaveBeenCalledWith(
        PLAN_ID,
        '2026-11-29',
        0,
        true,
        { sport: 'ride', workout: 'Long gravel ride' },
        'en',
      ),
    );

    await user.click(within(check).getByTestId('workspace-check-review-2026-12-06-0'));
    expect(await screen.findByRole('dialog', { name: i18n.t('plan.editor.editTitle', { date: 'Sun, Dec 6' }) })).toBeInTheDocument();
  });

  it('moves focus to the next session the check names once one is kept, and past the check after the last', async () => {
    const kept: string[] = [];
    api.setSubjectDraftDayCoachCall.mockImplementation(async (...args: unknown[]) => {
      kept.push(args[1] as string);
      api.getSubjectPlan.mockResolvedValue(withCalls(kept));
      return withCalls(kept);
    });
    const user = userEvent.setup();
    renderWorkspace();
    const check = await screen.findByTestId('workspace-prepublish');

    await user.click(within(check).getByTestId('workspace-check-keep-2026-11-29-0'));
    await waitFor(() => expect(screen.queryByTestId('workspace-check-2026-11-29-0')).not.toBeInTheDocument());
    expect(screen.getByTestId('workspace-check-keep-2026-12-06-0')).toHaveFocus();

    await user.click(screen.getByTestId('workspace-check-keep-2026-12-06-0'));
    await waitFor(() => expect(screen.queryByTestId('workspace-prepublish')).not.toBeInTheDocument());
    expect(screen.getByTestId('workspace-calendar-section')).toHaveFocus();
    expect(kept).toEqual(['2026-11-29', '2026-12-06']);
  });

  it('lists the history newest first, the coach as "you" and the agent by its name', async () => {
    renderWorkspace();

    const events = await screen.findAllByTestId('workspace-history-event');
    expect(events).toHaveLength(2);
    expect(events[0]).toHaveTextContent('Dravr');
    expect(events[0]).toHaveTextContent(i18n.t('plan.workspace.history.agentDraft'));
    expect(events[1]).toHaveTextContent(i18n.t('chat.senderYou'));
    expect(events[1]).toHaveTextContent(i18n.t('plan.workspace.history.dayAdd'));
    expect(api.getSubjectPlanHistory.mock.calls[0][0]).toBe(PLAN_ID);
  });

  it('draws the plan thread under the agent’s tab with one-line plan cards, and the room alone under Groupe', async () => {
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    await user.click(screen.getByRole('tab', { name: 'Dravr' }));
    const plan = within(screen.getByTestId('workspace-panel-dravr')).getByTestId('embedded-chat-stub');
    expect(plan).toHaveAttribute('data-conversation', PLAN_ID);
    expect(plan).toHaveAttribute('data-plan-surface', 'workspace');
    expect(screen.getByRole('tab', { name: 'Dravr' })).toHaveAttribute('aria-selected', 'true');

    await user.click(await screen.findByRole('tab', { name: i18n.t('app.group') }));
    const roomThread = within(screen.getByTestId('workspace-panel-group')).getByTestId('embedded-chat-stub');
    expect(roomThread).toHaveAttribute('data-conversation', ROOM_ID);
    expect(roomThread).toHaveAttribute('data-private-lane', 'false');
  });

  it('says when the athlete has no room of their own, and writes in their group instead', async () => {
    api.getGroup.mockResolvedValue(group(false));
    const user = userEvent.setup();
    const { routes } = renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    await user.click(await screen.findByRole('tab', { name: i18n.t('app.group') }));
    expect(screen.getByTestId('workspace-no-room')).toHaveTextContent(i18n.t('plan.workspace.noRoom', { name: LEA.display_name }));
    await user.click(screen.getByTestId('workspace-write-group'));
    expect(routes.writeToGroup).toHaveBeenCalledWith(ATHLETE, null);
  });

  it('says the room could not be read when a group read fails, opens no plan thread under a guess, and asks again', async () => {
    api.getConversations.mockResolvedValue({ conversations: [room], total: 1 });
    api.getGroup.mockRejectedValueOnce(new Error('network down'));
    const user = userEvent.setup();
    renderWorkspace();

    const failed = await screen.findByTestId('workspace-thread-failed');
    expect(failed).toHaveTextContent(i18n.t('humanCoach.planThread.openError'));
    expect(api.openAthleteThread).not.toHaveBeenCalled();
    await user.click(screen.getByRole('tab', { name: i18n.t('app.group') }));
    expect(screen.getByTestId('workspace-room-failed')).toHaveTextContent(i18n.t('humanCoach.groupRoom.openError'));

    await user.click(screen.getByTestId('workspace-room-retry'));
    await waitFor(() => expect(api.openAthleteThread).toHaveBeenCalledWith(LEA.user_id, GROUP.id));
    expect(await screen.findByTestId('workspace-calendar')).toBeInTheDocument();
    const roomThread = within(screen.getByTestId('workspace-panel-group')).getByTestId('embedded-chat-stub');
    expect(roomThread).toHaveAttribute('data-conversation', ROOM_ID);
  });

  it('opens the plan thread when the coach holds none on the athlete', async () => {
    api.getConversations.mockResolvedValue({ conversations: [room], total: 1 });
    renderWorkspace();

    await waitFor(() => expect(api.openAthleteThread).toHaveBeenCalledWith(LEA.user_id, GROUP.id));
    expect(await screen.findByTestId('workspace-calendar')).toBeInTheDocument();
    expect(api.openAthleteThread).toHaveBeenCalledTimes(1);
  });

  it('stands its publish disabled with no draft open, and offers the agent’s start with no plan at all', async () => {
    api.getSubjectPlan.mockResolvedValue({ ...subjectPlan(false), live: null });
    const user = userEvent.setup();
    renderWorkspace();

    expect(await screen.findByTestId('workspace-publish-idle')).toBeDisabled();
    expect(screen.getByTestId('workspace-no-plan')).toHaveTextContent(i18n.t('plan.workspace.noPlan', { name: LEA.display_name }));
    await user.click(screen.getByTestId('workspace-draft-plan'));
    expect(screen.getByRole('tab', { name: 'Dravr' })).toHaveAttribute('aria-selected', 'true');
  });

  it('goes back to the athlete in À traiter', async () => {
    const user = userEvent.setup();
    const { navigate } = renderWorkspace();
    await user.click(screen.getByTestId('workspace-back'));
    expect(navigate).toHaveBeenCalledWith('home/athlete/u-lea');
  });

  it('below lg, lists the days and opens the panel as a sheet from each tab', async () => {
    screenSize.desktop = false;
    const user = userEvent.setup();
    renderWorkspace();
    await screen.findByTestId('workspace-calendar');

    expect(screen.queryByTestId('workspace-aside')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('workspace-open-dravr'));
    const sheet = await screen.findByTestId('workspace-panel-sheet');
    expect(within(sheet).getByTestId('embedded-chat-stub')).toHaveAttribute('data-conversation', PLAN_ID);

    // The plan card's way back closes the sheet onto the calendar.
    await user.click(within(sheet).getByRole('button', { name: 'stand-in plan card' }));
    await waitFor(() => expect(screen.queryByTestId('workspace-panel-sheet')).not.toBeInTheDocument());
    expect(screen.getByTestId('workspace-calendar-section')).toHaveFocus();
  });
});
