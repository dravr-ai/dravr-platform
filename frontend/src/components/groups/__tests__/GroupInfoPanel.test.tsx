// ABOUTME: Tests for Group info inside chat, ordered by its reader — the owner offered the seat, the coach, an athlete
// ABOUTME: Consent bound to the caller's OWN row, autosaved settings, the folded week and room, quiet exits behind confirms
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, cleanup, render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import GroupInfoPanel from '../GroupInfoPanel';
import { ToastProvider } from '../../ui';
import { ThemeProvider } from '../../../hooks/useTheme';
import type {
  CoachConsentsResponse,
  CoachingGroup,
  DelegatedConnection,
  GroupChatConnectCode,
  GroupChatConnection,
  GroupMember,
  WorkoutPlan,
} from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import { UnitsContext } from '@pierre/ui-logic';
import { formatMessageTime, type DistanceUnit } from '@pierre/chat-utils';

const CALLER_ID = 'user-caller';
const OTHER_ID = 'user-other';
const GROUP_ID = 'group-1';
const COACH_ID = 'user-coach';
/** A coach id shaped as the server serialises it, to prove it never reaches the screen. */
const COACH_UUID = '5b0c1e9e-8a61-4c1f-9d2e-3f4a5b6c7d8e';

/** What the caller lets the group's coach do: nothing granted, nothing offered unless overridden. */
function coachConsents(overrides: Partial<CoachConsentsResponse> = {}): CoachConsentsResponse {
  return {
    calendar_write: false,
    calendar_write_at: null,
    calendar_write_supported: false,
    wellness_read: false,
    wellness_read_at: null,
    wellness_read_supported: false,
    wellness_read_skipped: null,
    wellness_notice_due: false,
    ...overrides,
  };
}

/** The room's chat-app connection: none, and the caller may not change it, unless overridden. */
function chatConnection(overrides: Partial<GroupChatConnection> = {}): GroupChatConnection {
  return {
    connected: false,
    channel: null,
    chat_title: null,
    connected_at: null,
    can_manage: false,
    ...overrides,
  };
}

/** A connected room, as the server reads it back once the bot joined the chat. */
const CONNECTED: Partial<GroupChatConnection> = {
  connected: true,
  channel: 'telegram',
  chat_title: 'Sunday Riders',
  connected_at: '2026-11-19T10:00:00Z',
};

/** A connect code that is still good for ten minutes. */
function connectCode(overrides: Partial<GroupChatConnectCode> = {}): GroupChatConnectCode {
  return {
    channel: 'telegram',
    code: 'K7Q2ZP',
    deep_link_url: 'https://t.me/dravr_bot?startgroup=K7Q2ZP',
    command: '/group link K7Q2ZP',
    qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    expires_at: new Date(Date.now() + 10 * 60_000).toISOString(),
    ...overrides,
  };
}

function link(overrides: Partial<DelegatedConnection> = {}): DelegatedConnection {
  return {
    id: 'dc-1',
    group_id: GROUP_ID,
    provider: 'trainingpeaks',
    coach_user_id: COACH_ID,
    coach_display_name: 'Casey Coach',
    member_user_id: CALLER_ID,
    member_display_name: 'caller@example.com',
    provider_athlete_id: '900001',
    provider_athlete_name: 'Alex Athlete',
    status: 'proposed',
    proposed_at: '2026-09-24T08:00:00Z',
    confirmed_at: null,
    read_refused: null,
    calendar_write: false,
    calendar_write_supported: false,
    wellness_read: false,
    wellness_read_supported: false,
    wellness_read_skipped: null,
    ...overrides,
  };
}

vi.mock('../../../services/api', () => ({
  groupsApi: {
    getGroup: vi.fn(),
    listMembers: vi.fn(),
    getStats: vi.fn(),
    getPermissions: vi.fn(),
    getWeeklyReport: vi.fn(),
    getHealthFlags: vi.fn(),
    updateGroup: vi.fn(),
    updatePeerConsent: vi.fn(),
    getCoachSharing: vi.fn(),
    getCoachReads: vi.fn(),
    updateCoachSharing: vi.fn(),
    leaveGroup: vi.fn(),
    deleteGroup: vi.fn(),
    removeCoach: vi.fn(),
    claimCoach: vi.fn(),
    listInvites: vi.fn(),
    createInvite: vi.fn(),
    deactivateInvite: vi.fn(),
    removeMember: vi.fn(),
    updateMemberRole: vi.fn(),
    getTranscript: vi.fn(),
    listDelegatedConnections: vi.fn(),
    getDelegationRoster: vi.fn(),
    proposeDelegatedConnection: vi.fn(),
    confirmDelegatedConnection: vi.fn(),
    getCoachConsents: vi.fn(),
    updateCoachConsents: vi.fn(),
    endDelegatedConnection: vi.fn(),
    getChatConnection: vi.fn(),
    createChatConnection: vi.fn(),
    deleteChatConnection: vi.fn(),
  },
  // The seat block's coaching-platform rows reach these only when tapped.
  oauthApi: {
    sciotteCancel: vi.fn(),
    authorizeUrl: vi.fn(),
  },
  providersApi: {
    getProvidersStatus: vi.fn(),
  },
  userApi: {
    getOnboardingStatus: vi.fn(),
    getCoachAccessRequest: vi.fn(),
    requestCoachAccess: vi.fn(),
    cancelCoachAccessRequest: vi.fn(),
  },
  chatApi: {
    openAthleteThread: vi.fn(),
    getAthletePlan: vi.fn(),
    getAthletePhysiology: vi.fn(),
    updateAthletePhysiologyAnchor: vi.fn(),
  },
}));

/** The signed-in caller; a test flips `manages_roster` to hold coach access. */
const authUser: { id: string; email: string; manages_roster?: boolean } = {
  id: CALLER_ID,
  email: 'caller@example.com',
};

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: authUser }),
}));

const { chatApi, groupsApi, providersApi, userApi } = await import('../../../services/api');

/** The onboarding status, answering whether the caller coaches others. */
function onboardingStatus(coachesOthers: boolean) {
  return {
    needs_provider_connection: false,
    pillars_covered: 0,
    pillars_total: 7,
    onboarding_complete: false,
    steps: [],
    chosen_channel: null,
    coaches_others: coachesOthers,
  };
}

function sampleGroup(overrides: Partial<CoachingGroup> = {}): CoachingGroup {
  return {
    id: GROUP_ID,
    tenant_id: 'tenant-a',
    name: 'Marathon Squad',
    description: 'Sunday long runs',
    agent_id: 'coach-1',
    agent_title: 'Marathon Coach',
    agent_handle: 'marathon-coach',
    coach_user_id: null,
    coach_display_name: null,
    owner_id: CALLER_ID,
    max_members: 10,
    peer_data_sharing: true,
    respond_mode: 'all',
    digest_mode: 'off',
    is_active: true,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

function member(overrides: Partial<GroupMember> = {}): GroupMember {
  return {
    id: 'membership-1',
    group_id: GROUP_ID,
    user_id: CALLER_ID,
    role: 'owner',
    peer_sharing_consent: false,
    consent_given_at: null,
    joined_at: '2026-01-01T00:00:00Z',
    display_name: 'Caller',
    ...overrides,
  } as GroupMember;
}

function renderPanel(units: DistanceUnit = 'metric') {
  const onMembershipEnded = vi.fn();
  const onOpenConversation = vi.fn();
  const onOpenCoachHome = vi.fn();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const utils = render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <UnitsContext.Provider value={units}>
          <ToastProvider>
            <GroupInfoPanel
              groupId={GROUP_ID}
              onMembershipEnded={onMembershipEnded}
              onOpenConversation={onOpenConversation}
              onOpenCoachHome={onOpenCoachHome}
            />
          </ToastProvider>
        </UnitsContext.Provider>
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { ...utils, onMembershipEnded, onOpenConversation, onOpenCoachHome };
}

describe('GroupInfoPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    authUser.manages_roster = undefined;
    vi.mocked(userApi.getOnboardingStatus).mockResolvedValue(onboardingStatus(false));
    vi.mocked(userApi.getCoachAccessRequest).mockResolvedValue({ request: null });
    vi.mocked(providersApi.getProvidersStatus).mockResolvedValue({ providers: [] } as never);
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup());
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(coachConsents());
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [
        // The other member is listed FIRST on purpose: binding the switch to
        // the first active row handed this caller someone else's consent.
        member({
          id: 'membership-2',
          user_id: OTHER_ID,
          role: 'member',
          peer_sharing_consent: true,
          display_name: 'Other',
        }),
        member(),
      ],
    });
    vi.mocked(groupsApi.getStats).mockResolvedValue({
      stats: {
        total_members: 3,
        active_members: 2,
        avg_weekly_volume_km: 41.5,
        avg_ctl: 55,
        flagged_members: 0,
        weekly_trend: 'stable',
      },
      scope: { visible_members: 3, roster_members: 3 },
    });
    vi.mocked(groupsApi.getPermissions).mockResolvedValue({
      can_create: true,
      policy: 'everyone',
      weekly_digest: true,
    });
    vi.mocked(groupsApi.getWeeklyReport).mockResolvedValue({
      report: {
        fresh_members: [{ user_id: CALLER_ID, display_name: 'Caller', form_pct: 12.4, tsb: 8.6 }],
        stats: {
          total_members: 2,
          active_members: 2,
          avg_weekly_volume_km: 41.5,
          avg_ctl: 55,
          flagged_members: 1,
          weekly_trend: 'stable',
        },
      },
      scope: { visible_members: 2, roster_members: 2 },
    });
    vi.mocked(groupsApi.getHealthFlags).mockResolvedValue({
      flags: [
        {
          user_id: OTHER_ID,
          display_name: 'Other',
          flag_type: 'inactive',
          severity: 'warning',
          evidence: { kind: 'inactive_days', days: 11 },
        },
        {
          user_id: CALLER_ID,
          display_name: 'Caller',
          flag_type: 'volume_drop',
          severity: 'warning',
          evidence: { kind: 'volume_below_group', pct_below: 35 },
        },
      ],
      total: 2,
      scope: { visible_members: 2, roster_members: 2 },
    });
    vi.mocked(groupsApi.updatePeerConsent).mockResolvedValue(undefined);
    vi.mocked(groupsApi.getCoachSharing).mockResolvedValue({ consent: true, assistant_notice_due: false });
    vi.mocked(groupsApi.getCoachReads).mockResolvedValue({ reads: [], retention_months: 12 });
    vi.mocked(groupsApi.updateCoachSharing).mockImplementation(async (_groupId, request) => ({
      ...request,
      assistant_notice_due: false,
    }));
    vi.mocked(groupsApi.updateGroup).mockResolvedValue(sampleGroup());
    vi.mocked(groupsApi.deleteGroup).mockResolvedValue(undefined);
    vi.mocked(groupsApi.listInvites).mockResolvedValue({ invites: [] });
    vi.mocked(groupsApi.getTranscript).mockResolvedValue({ entries: [], total: 0 });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({
      connections: [],
      total: 0,
      viewer: 'member',
    });
    vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection());
  });

  it('names the group, its description and its roster size', async () => {
    renderPanel();

    expect(await screen.findByTestId('group-info-name')).toHaveTextContent('Marathon Squad');
    expect(screen.getByTestId('group-info-description')).toHaveTextContent('Sunday long runs');
    expect(await screen.findByText('Members: 2')).toBeInTheDocument();
  });

  describe('ordered by who reads it (carnet#1527)', () => {
    it('leads the owner offered the seat with the block, then who is here, then the folded rows and the quiet exit', async () => {
      renderPanel();

      const block = await screen.findByTestId('coach-seat-block');
      const people = screen.getByTestId('group-info-people');
      const more = screen.getByTestId('group-info-more');
      const exit = screen.getByTestId('group-info-delete');
      expect(block.compareDocumentPosition(people) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(people.compareDocumentPosition(more) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(more.compareDocumentPosition(exit) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      // The invite link sits under who is here, not in the settings.
      expect(within(people).getByTestId('invite-manager')).toBeInTheDocument();
    });

    it("leads the group's coach with their athlete, then the planning connections, then the agent", async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
      );
      vi.mocked(groupsApi.listMembers).mockResolvedValue({
        members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Alice Runner' })],
      });
      vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
      vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
      renderPanel();

      const athletes = await screen.findByTestId('group-info-athletes');
      expect(within(athletes).getByRole('heading', { name: 'Your athlete' })).toBeInTheDocument();
      const connections = await screen.findByTestId('delegation-section');
      const agent = screen.getByTestId('group-info-agent-section');
      expect(athletes.compareDocumentPosition(connections) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(connections.compareDocumentPosition(agent) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      // The agent's section says how it replies, read in place.
      expect(within(agent).getByTestId('group-info-agent-replies')).toHaveTextContent('Replies in the roomTo every message');
      expect(screen.queryByTestId('group-info-people')).toBeNull();
    });

    it('leads an athlete with who is here and what they see, then their own sharing', async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ owner_id: OTHER_ID, coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
      );
      vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
      renderPanel();

      const people = await screen.findByTestId('group-info-people');
      const sharing = await screen.findByTestId('group-info-sharing');
      expect(people.compareDocumentPosition(sharing) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(screen.queryByTestId('coach-seat-block')).toBeNull();
      expect(screen.queryByTestId('group-info-athletes')).toBeNull();
    });
  });

  it('names the AI agent and the human coach above the members for a plain member, and says what each sees', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ owner_id: OTHER_ID, coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [
        member({ role: 'member' }),
        member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Other' }),
      ],
    });
    renderPanel();

    const agent = await screen.findByTestId('group-info-agent');
    expect(within(agent).getByText('Marathon Coach')).toBeInTheDocument();
    expect(within(agent).getByTestId('group-info-agent-handle')).toHaveTextContent('@marathon-coach');
    expect(within(agent).getByText('AI agent')).toBeInTheDocument();
    const coach = screen.getByTestId('group-info-coach-row');
    expect(within(coach).getByText('Casey Coach')).toBeInTheDocument();
    expect(within(coach).getByText('Coach')).toBeInTheDocument();
    expect(within(coach).queryByText('(you)')).toBeNull();
    expect(screen.queryByTestId('group-info-no-coach')).toBeNull();
    // The header names the coach instead of only saying one is attached.
    expect(screen.getByTestId('group-info-coach-badge')).toHaveTextContent('Coach Casey Coach');
    // Both rows lead the roster: they sit before the members.
    const list = await screen.findByTestId('member-list');
    expect(coach.compareDocumentPosition(agent) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(agent.compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // What each of them sees, as the group really is: sharing on, the agent replying to every message.
    expect(await screen.findByTestId('who-sees-coach')).toHaveTextContent(
      'Casey Coach sees your training, your plan and your messages here.',
    );
    expect(screen.getByTestId('who-sees-agent')).toHaveTextContent('Marathon Coach reads the room and replies to every message.');
    expect(screen.getByTestId('who-sees-plan')).toHaveTextContent(
      'Casey Coach prepares your plan with Marathon Coach in a private thread. You see what Casey Coach publishes.',
    );
  });

  it("says what the coach stops seeing once the athlete's sharing is off, and how a mention-only agent answers", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ owner_id: OTHER_ID, coach_user_id: COACH_ID, coach_display_name: 'Casey Coach', respond_mode: 'mentions' }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
    vi.mocked(groupsApi.getCoachSharing).mockResolvedValue({ consent: false, assistant_notice_due: false });
    renderPanel();

    expect(await screen.findByTestId('who-sees-coach')).toHaveTextContent(
      "Casey Coach doesn't see your training, your plan or your messages here while your sharing is off.",
    );
    expect(screen.getByTestId('who-sees-agent')).toHaveTextContent(
      'Marathon Coach follows the room and replies when you mention it with @marathon-coach.',
    );
  });

  it('says the group has no human coach yet when none is attached', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
    renderPanel();

    expect(await screen.findByTestId('group-info-no-coach')).toHaveTextContent(
      'No human coach in this group yet.',
    );
    expect(screen.queryByTestId('group-info-coach-row')).toBeNull();
    expect(screen.queryByTestId('group-info-coach-badge')).toBeNull();
    // With no coach there is nothing a coach sees and no plan thread to describe.
    expect(screen.queryByTestId('who-sees-coach')).toBeNull();
    expect(screen.queryByTestId('who-sees-plan')).toBeNull();
    expect(screen.getByTestId('who-sees-agent')).toBeInTheDocument();
  });

  describe('coach seat (carnet#738, carnet#1520)', () => {
    const pending = {
      id: 'req-1',
      user_id: CALLER_ID,
      group_id: GROUP_ID,
      group_tenant_id: 'tenant-a',
      status: 'pending' as const,
      created_at: '2026-10-07T12:00:00Z',
      decided_at: null,
      decided_by: null,
    };
    const declined = {
      ...pending,
      status: 'declined' as const,
      decided_at: '2026-10-08T09:00:00Z',
      decided_by: 'admin-1',
    };

    /** The owner offered the seat: they coach others and hold no coach access. */
    function offerSeat() {
      vi.mocked(userApi.getOnboardingStatus).mockResolvedValue(onboardingStatus(true));
    }

    it('leads the drawer with the seat block, whose filled request names this group', async () => {
      offerSeat();
      vi.mocked(userApi.requestCoachAccess).mockResolvedValue({ request: pending });
      renderPanel();

      const block = await screen.findByTestId('coach-seat-block');
      expect(block).toHaveAttribute('data-state', 'to_request');
      expect(within(block).getByRole('heading', { name: "You're not this group's coach yet" })).toBeInTheDocument();
      // First in the drawer: before the consent and the roster.
      const consent = screen.getByTestId('peer-consent-card');
      expect(block.compareDocumentPosition(consent) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      const button = within(block).getByTestId('coach-access-request');
      expect(button).toHaveClass('btn-primary');
      await userEvent.click(button);

      await waitFor(() => expect(userApi.requestCoachAccess).toHaveBeenCalledWith(GROUP_ID));
      // The block turns into the dated three-step timeline, with no button.
      expect(await within(block).findByTestId('coach-access-pending')).toBeInTheDocument();
      expect(block).toHaveAttribute('data-state', 'pending');
      expect(within(block).getByTestId('coach-seat-step-sent')).toHaveTextContent('Request sent');
      expect(within(block).getByTestId('coach-seat-step-review')).toHaveAttribute('aria-current', 'step');
      // No wait time is promised; the answer comes here and by email (carnet#1521).
      expect(within(block).getByTestId('coach-seat-step-review')).toHaveTextContent(/here and by email/i);
      expect(block.textContent).not.toMatch(/24/);
      expect(within(block).queryByTestId('coach-access-request')).toBeNull();
      // The waiting request can be withdrawn from where it is shown.
      expect(within(block).getByTestId('coach-access-cancel')).toHaveTextContent('Cancel request');
    });

    it('withdraws the waiting request and offers it again at once', async () => {
      offerSeat();
      vi.mocked(userApi.getCoachAccessRequest).mockResolvedValue({ request: pending });
      vi.mocked(userApi.cancelCoachAccessRequest).mockResolvedValue({ request: null });
      renderPanel();

      const block = await screen.findByTestId('coach-seat-block');
      await waitFor(() => expect(block).toHaveAttribute('data-state', 'pending'));
      await userEvent.click(within(block).getByTestId('coach-access-cancel'));

      await waitFor(() => expect(userApi.cancelCoachAccessRequest).toHaveBeenCalled());
      await waitFor(() => expect(block).toHaveAttribute('data-state', 'to_request'));
      expect(within(block).getByTestId('coach-access-cancelled')).toHaveTextContent('Request cancelled');
      expect(within(block).getByTestId('coach-access-request')).toBeEnabled();
    });

    it('shows the reason the team gave with a decline, beside "Ask again"', async () => {
      offerSeat();
      vi.mocked(userApi.getCoachAccessRequest).mockResolvedValue({
        request: {
          ...pending,
          status: 'declined',
          decided_at: '2026-11-13T09:00:00Z',
          decline_reason: 'No coaching account we could verify.',
        },
      });
      renderPanel();

      const block = await screen.findByTestId('coach-seat-block');
      await waitFor(() => expect(block).toHaveAttribute('data-state', 'declined'));
      expect(within(block).getByTestId('coach-access-decline-reason')).toHaveTextContent(
        'Reason: No coaching account we could verify.',
      );
      expect(within(block).getByTestId('coach-access-request')).toHaveTextContent('Ask again');
    });

    it("is the drawer's only filled button while it is offered, the settings included", async () => {
      offerSeat();
      const user = userEvent.setup();
      renderPanel();

      await screen.findByTestId('coach-seat-block');
      const panel = screen.getByTestId('group-info-panel');
      await within(panel).findByTestId('invite-new');
      expect(panel.querySelectorAll('.btn-primary')).toHaveLength(1);
      // The settings save as they change: opening them adds no Save button.
      await user.click(within(panel).getByRole('button', { name: /Group settings/ }));
      expect(await within(panel).findByTestId('group-info-settings')).toBeInTheDocument();
      expect(panel.querySelectorAll('.btn-primary')).toHaveLength(1);
      expect(within(panel).queryByRole('button', { name: /save/i })).toBeNull();
    });

    it('offers TrainingPeaks and Intervals.icu as identical plain rows', async () => {
      offerSeat();
      vi.mocked(providersApi.getProvidersStatus).mockResolvedValue({
        providers: [
          { provider: 'strava', display_name: 'Strava', description: 'Runs and rides', connected: false, requires_oauth: true },
          { provider: 'sciotte_trainingpeaks', display_name: 'TrainingPeaks', description: 'Workouts and load', connected: false, requires_oauth: false },
          { provider: 'intervals_icu', display_name: 'Intervals.icu', description: 'Activities and calendar', connected: false, requires_oauth: false },
        ],
      } as never);
      renderPanel();

      const routes = await screen.findByTestId('coach-seat-routes');
      const tp = await within(routes).findByTestId('provider-card-sciotte_trainingpeaks');
      const icu = within(routes).getByTestId('provider-card-intervals_icu');
      expect(within(routes).queryByTestId('provider-card-strava')).toBeNull();
      // Logo, name, Connect — no per-route line on either.
      expect(tp).toHaveTextContent(/^TrainingPeaksConnect$/);
      expect(icu).toHaveTextContent(/^Intervals\.icuConnect$/);
    });

    it('shows a request already waiting as the timeline and the roster line, never the button', async () => {
      offerSeat();
      vi.mocked(userApi.getCoachAccessRequest).mockResolvedValue({ request: pending });
      renderPanel();

      expect(await screen.findByTestId('coach-access-pending')).toBeInTheDocument();
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
      await waitFor(() =>
        expect(screen.getByTestId('group-info-coach-seat')).toHaveTextContent('You · coach access pending'),
      );
    });

    it('dates a declined request and offers to ask again', async () => {
      offerSeat();
      vi.mocked(userApi.getCoachAccessRequest).mockResolvedValue({ request: declined });
      vi.mocked(userApi.requestCoachAccess).mockResolvedValue({ request: pending });
      renderPanel();

      const block = await screen.findByTestId('coach-seat-block');
      await waitFor(() => expect(block).toHaveAttribute('data-state', 'declined'));
      expect(within(block).getByTestId('coach-access-declined')).toHaveTextContent(/Declined on .*2026\. You can ask again\./);
      await userEvent.click(within(block).getByRole('button', { name: 'Ask again' }));
      await waitFor(() => expect(userApi.requestCoachAccess).toHaveBeenCalledWith(GROUP_ID));
    });

    it('never tells the owner about to coach that the group has none, nor offers a coach invite or coach sharing', async () => {
      offerSeat();
      vi.mocked(groupsApi.listInvites).mockResolvedValue({ invites: [] });
      renderPanel();

      expect(await screen.findByTestId('group-info-coach-seat')).toHaveTextContent('You · not the coach yet');
      expect(screen.queryByTestId('group-info-no-coach')).toBeNull();
      const panel = screen.getByTestId('group-info-panel');
      expect(panel).not.toHaveTextContent('No human coach');
      expect(panel).not.toHaveTextContent('Create a Coach invite');
      expect(screen.queryByTestId('coach-sharing-row')).toBeNull();
      expect(panel).not.toHaveTextContent(/pending/i);
      // The invite form offers member invites only.
      await userEvent.click(await within(panel).findByRole('button', { name: /New invite/i }));
      expect(within(panel).queryByLabelText('Invite type')).toBeNull();
    });

    it('asks the owner with nobody to coach yet to invite their athlete, right under who is here', async () => {
      offerSeat();
      vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member()] });
      renderPanel();

      const people = await screen.findByTestId('group-info-people');
      expect(await within(people).findByText('Invite your athlete')).toBeInTheDocument();
      // The owner reads themselves in the coach slot, never twice.
      expect(within(people).queryByTestId('member-list')).toBeNull();
    });

    it('offers the seat to an owner who never said they coach others (carnet#1522)', async () => {
      // The onboarding answer is "athlete"; owning a coachless group without
      // coach access is what offers the request.
      renderPanel();

      const block = await screen.findByTestId('coach-seat-block');
      expect(block).toHaveAttribute('data-state', 'to_request');
      expect(within(block).getByTestId('coach-access-request')).toBeEnabled();
      expect(userApi.getOnboardingStatus).not.toHaveBeenCalled();
    });

    it('offers nothing to an owner who already holds coach access', async () => {
      authUser.manages_roster = true;
      offerSeat();
      renderPanel();

      await screen.findByTestId('group-info-no-coach');
      expect(screen.queryByTestId('coach-seat-block')).toBeNull();
      expect(userApi.getOnboardingStatus).not.toHaveBeenCalled();
    });

    it('offers the owner who holds coach access the one way to coach their coachless group (carnet#1650)', async () => {
      authUser.manages_roster = true;
      offerSeat();
      vi.mocked(groupsApi.claimCoach).mockResolvedValue({
        group: sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach' }),
      });
      renderPanel();

      const block = await screen.findByTestId('group-info-claim-coach');
      expect(block).toHaveTextContent('This group has no coach yet');
      expect(block).toHaveTextContent("You'll see the training of the athletes who share it with you.");
      expect(screen.queryByTestId('coach-seat-block')).toBeNull();

      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach' }),
      );
      await userEvent.click(within(block).getByTestId('group-info-claim-coach-action'));

      await waitFor(() => expect(groupsApi.claimCoach).toHaveBeenCalledExactlyOnceWith(GROUP_ID));
      // The group is re-read, and the offer gives way to the coach row.
      await waitFor(() => expect(screen.queryByTestId('group-info-claim-coach')).toBeNull());
      expect(await screen.findByText("You're now this group's coach.")).toBeInTheDocument();
    });

    it('says the claim failed and keeps the offer', async () => {
      authUser.manages_roster = true;
      offerSeat();
      vi.mocked(groupsApi.claimCoach).mockRejectedValue({ response: { status: 500, data: {} } });
      renderPanel();

      const block = await screen.findByTestId('group-info-claim-coach');
      await userEvent.click(within(block).getByTestId('group-info-claim-coach-action'));

      expect(await within(block).findByTestId('group-info-claim-coach-failed')).toHaveTextContent(
        "Couldn't make you the coach. Try again.",
      );
      expect(within(block).getByTestId('group-info-claim-coach-action')).toBeEnabled();
    });

    it('offers no claim to an owner without coach access: the seat request stays theirs', async () => {
      offerSeat();
      renderPanel();

      await screen.findByTestId('coach-seat-block');
      expect(screen.queryByTestId('group-info-claim-coach')).toBeNull();
    });

    it('offers no claim to a group that already has a coach', async () => {
      authUser.manages_roster = true;
      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
      );
      renderPanel();

      await screen.findByTestId('group-info-coach-row');
      expect(screen.queryByTestId('group-info-claim-coach')).toBeNull();
    });

    it('offers nothing to a plain member of a coachless group', async () => {
      offerSeat();
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
      vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
      renderPanel();

      await screen.findByTestId('group-info-no-coach');
      expect(screen.queryByTestId('coach-seat-block')).toBeNull();
      expect(userApi.getOnboardingStatus).not.toHaveBeenCalled();
    });
  });

  it('labels an agent the server could not resolve as the AI agent, with no handle', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ agent_title: null, agent_handle: null }),
    );
    renderPanel();

    const agent = await screen.findByTestId('group-info-agent');
    expect(within(agent).getAllByText('AI agent')).toHaveLength(2);
    expect(within(agent).queryByTestId('group-info-agent-handle')).toBeNull();
  });

  it("names the coach in their row, never prints their id, and detaches them only from the row's menu and its confirm", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: COACH_UUID, coach_display_name: 'Casey Coach' }),
    );
    vi.mocked(groupsApi.removeCoach).mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderPanel();

    expect(await screen.findByTestId('group-info-coach-name')).toHaveTextContent('Casey Coach');
    expect(screen.getByTestId('group-info-panel').textContent).not.toContain(COACH_UUID);
    expect(document.body.textContent).not.toContain(COACH_UUID);
    // No filled red on the row: removing the coach is behind its ⋯.
    expect(screen.queryByTestId('group-info-remove-coach')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'More actions for Casey Coach' }));
    await user.click(screen.getByTestId('group-info-remove-coach'));
    const confirm = await screen.findByRole('dialog');
    expect(confirm).toHaveTextContent('Remove the coach?');
    await user.click(within(confirm).getByRole('button', { name: 'Remove coach' }));
    await waitFor(() => expect(groupsApi.removeCoach).toHaveBeenCalledWith(GROUP_ID));
  });

  it('binds the consent switch to the caller own membership row', async () => {
    renderPanel();

    const toggle = await screen.findByTestId('peer-consent-switch');
    // The caller's own row has consent off; the other member's row has it on.
    expect((toggle as HTMLInputElement).checked).toBe(false);
  });

  it('sends the new consent value when the caller flips the switch', async () => {
    renderPanel();

    const toggle = await screen.findByTestId('peer-consent-switch');
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(groupsApi.updatePeerConsent).toHaveBeenCalledWith(GROUP_ID, { consent: true });
    });
    expect(groupsApi.updatePeerConsent).toHaveBeenCalledTimes(1);
  });

  it('asks nobody about peer sharing while the group shares nothing', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID, peer_data_sharing: false }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
    renderPanel();

    expect(await screen.findByTestId('coach-sharing-row')).toBeInTheDocument();
    expect(screen.queryByTestId('peer-consent-card')).toBeNull();
  });

  it("shows the caller's own coach sharing beside peer sharing, and stops it when flipped", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
    );
    renderPanel();

    const toggle = (await screen.findByTestId('coach-sharing-switch')) as HTMLInputElement;
    await waitFor(() => expect(toggle.checked).toBe(true));
    expect(groupsApi.getCoachSharing).toHaveBeenCalledWith(GROUP_ID);
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(groupsApi.updateCoachSharing).toHaveBeenCalledWith(GROUP_ID, { consent: false });
    });
    await waitFor(() => expect(toggle.checked).toBe(false));
    // Peer sharing is a separate choice the flip leaves alone.
    expect(groupsApi.updatePeerConsent).not.toHaveBeenCalled();
    // Beside it, the member's own record of what the coach read (carnet#1445).
    expect(await screen.findByTestId('coach-read-log')).toBeInTheDocument();
    expect(groupsApi.getCoachReads).toHaveBeenCalledWith(GROUP_ID);
  });

  // EFVP decision 9.2b: a sharing member who never answered the notice is told once,
  // as the wellness widening is, and either answer is a coach-sharing write.
  it("tells a sharing member who never answered of the coach's AI tools, and keeping it writes the sharing again", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
    );
    vi.mocked(groupsApi.getCoachSharing).mockResolvedValue({ consent: true, assistant_notice_due: true });
    renderPanel();

    const notice = await screen.findByTestId('coach-assistant-notice');
    expect(notice).toHaveAccessibleName(i18n.t('humanCoach.assistantNotice.title'));
    expect(notice).toHaveClass('alert-info');
    expect(notice).toHaveTextContent(i18n.t('humanCoach.assistantNotice.body', { coach: 'Casey Coach' }));
    // The sharing stands while the member decides.
    expect(screen.getByTestId('coach-sharing-switch')).toBeChecked();

    // The answers are the wellness notice's own, so both notices read alike.
    const keep = within(notice).getByTestId('coach-assistant-notice-keep');
    expect(keep).toHaveTextContent(i18n.t('delegation.wellnessWidenedKeep'));
    fireEvent.click(keep);

    await waitFor(() => expect(groupsApi.updateCoachSharing).toHaveBeenCalledWith(GROUP_ID, { consent: true }));
    await waitFor(() => expect(screen.queryByTestId('coach-assistant-notice')).toBeNull());
    expect(screen.getByTestId('coach-sharing-switch')).toBeChecked();
  });

  it("stops sharing from the coach's AI tools notice, ending the coach's access with it", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
    );
    vi.mocked(groupsApi.getCoachSharing).mockResolvedValue({ consent: true, assistant_notice_due: true });
    renderPanel();

    const notice = await screen.findByTestId('coach-assistant-notice');
    const stop = within(notice).getByTestId('coach-assistant-notice-stop');
    expect(stop).toHaveTextContent(i18n.t('delegation.wellnessRevokeConfirm'));
    fireEvent.click(stop);

    await waitFor(() => expect(groupsApi.updateCoachSharing).toHaveBeenCalledWith(GROUP_ID, { consent: false }));
    await waitFor(() => expect(screen.queryByTestId('coach-assistant-notice')).toBeNull());
    expect(screen.getByTestId('coach-sharing-switch')).not.toBeChecked();
  });

  it("tells nothing of the coach's AI tools to a member who already answered", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
    );
    renderPanel();

    expect(await screen.findByTestId('coach-sharing-switch')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-assistant-notice')).toBeNull();
  });

  it('offers coach sharing in a group with no coach yet, since joining granted it', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
    renderPanel();

    expect(await screen.findByTestId('coach-sharing-row')).toBeInTheDocument();
    // No coach has read anything here, so there is no record to show.
    expect(screen.queryByTestId('coach-read-log')).toBeNull();
    expect(groupsApi.getCoachReads).not.toHaveBeenCalled();
  });

  it('lists the others with their role, the viewer marked, one row each', async () => {
    // An owner who is not offered the seat (someone else's group to coach): the member view.
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
    renderPanel();

    const list = within(await screen.findByTestId('member-list'));
    expect(list.getByText('Caller')).toBeInTheDocument();
    expect(list.getByText('(you)')).toBeInTheDocument();
    expect(list.getByText('Other')).toBeInTheDocument();
    expect(list.getByText('Owner')).toBeInTheDocument();
    expect(list.getByText('Member')).toBeInTheDocument();
    expect(screen.queryByRole('table')).toBeNull();
  });

  it('folds settings, the week and the room history, and reads the room only once it is opened', async () => {
    const user = userEvent.setup();
    renderPanel();

    const room = await screen.findByRole('button', { name: 'Room history' });
    expect(room).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByRole('button', { name: /Group settings/ })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByRole('button', { name: 'This week' })).toHaveAttribute('aria-expanded', 'false');
    expect(groupsApi.getTranscript).not.toHaveBeenCalled();

    await user.click(room);
    expect(room).toHaveAttribute('aria-expanded', 'true');
    await waitFor(() => expect(groupsApi.getTranscript).toHaveBeenCalled());
  });

  it('saves the group name when the field is left, and only the name', async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: /Group settings/ }));
    const name = await screen.findByLabelText('Group name');
    await user.clear(name);
    await user.type(name, 'Harricana Squad');
    expect(groupsApi.updateGroup).not.toHaveBeenCalled();
    await user.tab();

    await waitFor(() => expect(groupsApi.updateGroup).toHaveBeenCalledWith(GROUP_ID, { name: 'Harricana Squad' }));
    expect(groupsApi.updateGroup).toHaveBeenCalledTimes(1);
  });

  it('puts an emptied name back instead of saving it', async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: /Group settings/ }));
    const name = await screen.findByLabelText<HTMLInputElement>('Group name');
    await user.clear(name);
    await user.tab();

    expect(name.value).toBe('Marathon Squad');
    expect(groupsApi.updateGroup).not.toHaveBeenCalled();
  });

  it('lets an admin choose where the weekly digest goes, saved the moment it changes, with the mode explained', async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: /Group settings/ }));
    const select = await screen.findByTestId('group-digest-mode');
    expect(select).toHaveValue('off');
    expect(within(select).getAllByRole('option').map((o) => o.textContent)).toEqual([
      'Off',
      'In the group chat',
      'Owner and admins only',
    ]);
    expect(screen.getByText('Nothing is sent.')).toBeInTheDocument();

    await user.selectOptions(select, 'managers');
    // Read from the catalogue: the hint's wording belongs to the corpus.
    expect(screen.getByText(i18n.getFixedT('en')('groups.digestManagersHint'))).toBeInTheDocument();
    await waitFor(() => expect(groupsApi.updateGroup).toHaveBeenCalledWith(GROUP_ID, { digest_mode: 'managers' }));
  });

  it('puts a refused setting back where the server still has it', async () => {
    vi.mocked(groupsApi.updateGroup).mockRejectedValue(new Error('Request failed with status code 500'));
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: /Group settings/ }));
    const sharing = await screen.findByTestId('group-settings-peer-sharing');
    expect(sharing).toBeChecked();
    await user.click(sharing);

    await waitFor(() => expect(groupsApi.updateGroup).toHaveBeenCalledWith(GROUP_ID, { peer_data_sharing: false }));
    await waitFor(() => expect(sharing).toBeChecked());
  });

  it('gives the attached coach the digest select alone and sends only the digest mode', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ owner_id: OTHER_ID, coach_user_id: CALLER_ID, digest_mode: 'chat' }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    const user = userEvent.setup();
    renderPanel();

    const settings = await screen.findByRole('button', { name: /Group settings/ });
    // The row says what is inside for this viewer: the weekly recap, nothing else.
    expect(settings).toHaveTextContent('Weekly recap');
    await user.click(settings);
    const select = await screen.findByTestId('group-digest-mode');
    expect(select).toHaveValue('chat');
    // The coach changes the digest and nothing else: no name, no respond mode.
    expect(screen.queryByLabelText('Group name')).toBeNull();
    expect(screen.queryByLabelText('Agent replies in the group chat')).toBeNull();

    await user.selectOptions(select, 'off');

    await waitFor(() =>
      expect(groupsApi.updateGroup).toHaveBeenCalledWith(GROUP_ID, { digest_mode: 'off' }),
    );
  });

  it('hides the digest select when the tenant tier sends no digest', async () => {
    vi.mocked(groupsApi.getPermissions).mockResolvedValue({
      can_create: true,
      policy: 'everyone',
      weekly_digest: false,
    });
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: /Group settings/ }));
    expect(await screen.findByLabelText('Group name')).toBeInTheDocument();
    expect(screen.queryByTestId('group-digest-mode')).toBeNull();
  });

  it('renders the weekly report and one health-flag row per flagged member, once the week is opened', async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: 'This week' }));
    expect(await screen.findByTestId('group-report-summary')).toHaveTextContent(
      '2/2 members active this week, averaging 41.5 km each.',
    );
    const highlights = screen.getAllByTestId('group-report-highlight');
    expect(highlights).toHaveLength(1);
    expect(highlights[0]).toHaveTextContent('Caller: fresh form (+12% of chronic load, TSB +9)');
    // The concerns are the health flags, one line per flag.
    const concerns = screen.getAllByTestId('group-report-concern');
    expect(concerns.map((c) => c.textContent)).toEqual([
      'Other: No activity for 11 days',
      'Caller: Weekly volume 35% below the group average',
    ]);
    // The deep-fatigue members are a magnitude, never an overtraining risk (Methodology framing).
    const recommendations = screen.getAllByTestId('group-report-recommendation');
    expect(recommendations.map((r) => r.textContent)).toEqual([
      'Members in deep fatigue, with form below -30% of their chronic load: 1. Worth a look at their recovery.',
      'Group volume is steady compared with last week.',
    ]);
    const rows = screen.getAllByTestId('group-health-flag-row');
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText('No activity for 11 days')).toBeInTheDocument();
    expect(screen.getByText('Health flags (2)')).toBeInTheDocument();
    expect(within(screen.getByTestId('group-info-stats')).getByText('of 3 total')).toBeInTheDocument();
    // Every member shares with the reader: nothing to say about scope.
    expect(screen.queryByTestId('group-info-scope')).toBeNull();
  });

  it('keeps the averages away below three members sharing with the reader, the report still there', async () => {
    vi.mocked(groupsApi.getStats).mockResolvedValue({
      stats: {
        total_members: 2,
        active_members: 2,
        avg_weekly_volume_km: 41.5,
        avg_ctl: 55,
        flagged_members: 0,
        weekly_trend: 'stable',
      },
      scope: { visible_members: 2, roster_members: 2 },
    });
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: 'This week' }));
    expect(await screen.findByTestId('group-report-summary')).toBeInTheDocument();
    expect(screen.queryByTestId('group-info-stats')).toBeNull();
  });

  it('shows a plain member no week at all while fewer than three share', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
    vi.mocked(groupsApi.getStats).mockResolvedValue({
      stats: {
        total_members: 2,
        active_members: 2,
        avg_weekly_volume_km: 41.5,
        avg_ctl: 55,
        flagged_members: 0,
        weekly_trend: 'stable',
      },
      scope: { visible_members: 2, roster_members: 2 },
    });
    renderPanel();

    await screen.findByTestId('group-info-leave');
    await waitFor(() => expect(groupsApi.getStats).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: 'This week' })).toBeNull();
  });

  it('says under the stats that they cover only the members who share with the reader, owner included', async () => {
    vi.mocked(groupsApi.getStats).mockResolvedValueOnce({
      stats: {
        total_members: 3,
        active_members: 2,
        avg_weekly_volume_km: 41.5,
        avg_ctl: 55,
        flagged_members: 0,
        weekly_trend: 'stable',
      },
      scope: { visible_members: 3, roster_members: 5 },
    });
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: 'This week' }));
    expect(await screen.findByTestId('group-info-scope')).toHaveTextContent(
      'This report covers only the members who share their training with you (3 of 5).',
    );
    expect(within(screen.getByTestId('group-info-stats')).getByText('of 3 total')).toBeInTheDocument();
  });

  it('prints the group volume in miles for a reader on imperial units', async () => {
    const user = userEvent.setup();
    renderPanel('imperial');

    await user.click(await screen.findByRole('button', { name: 'This week' }));
    expect(await screen.findByTestId('group-report-summary')).toHaveTextContent(
      '2/2 members active this week, averaging 25.8 mi each.',
    );
    const stats = within(screen.getByTestId('group-info-stats'));
    // 41.5 km is 25.8 mi, with its unit beside it.
    expect(stats.getByText('25.8')).toBeInTheDocument();
    expect(stats.getByText('mi')).toBeInTheDocument();
    expect(stats.queryByText('km')).toBeNull();
  });

  describe('in French', () => {
    beforeEach(async () => {
      await i18n.changeLanguage('fr');
    });

    afterEach(async () => {
      // Unmount first: switching the language under a mounted panel re-renders
      // it outside act().
      cleanup();
      await i18n.changeLanguage('en');
    });

    it('phrases the flags, the report and the decimals in French', async () => {
      // An admin of someone else's group: no seat to offer, so the coach slot
      // reads the group's own line.
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
      const user = userEvent.setup();
      renderPanel();

      await user.click(await screen.findByRole('button', { name: 'Cette semaine' }));
      expect(await screen.findByTestId('group-report-summary')).toHaveTextContent(
        '2/2 membres actifs cette semaine, 41,5 km en moyenne par membre.',
      );
      const rows = screen.getAllByTestId('group-health-flag-row');
      expect(within(rows[0]).getByText('Aucune activité depuis 11 jours')).toBeInTheDocument();
      expect(within(rows[1]).getByText('Volume hebdo 35 % sous la moyenne du groupe')).toBeInTheDocument();
      expect(screen.getAllByTestId('group-report-concern')[0]).toHaveTextContent(
        'Other : Aucune activité depuis 11 jours',
      );
      expect(screen.getByTestId('group-report-highlight')).toHaveTextContent(
        'Caller : forme fraîche (+12 % de sa charge chronique, TSB +9)',
      );
      const stats = within(screen.getByTestId('group-info-stats'));
      expect(stats.getByText('sur 3 au total')).toBeInTheDocument();
      // The panel's own chrome reads French too: the member count, the
      // caller's marker and the folded rows.
      expect(screen.getByText('Membres : 2')).toBeInTheDocument();
      expect(screen.getByText('(toi)')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Historique du salon' })).toBeInTheDocument();
      expect(screen.getByTestId('group-info-delete')).toHaveTextContent('Supprimer ce groupe…');
      expect(stats.getByText('41,5')).toBeInTheDocument();
      // Who runs the group reads French as well.
      expect(within(screen.getByTestId('group-info-agent')).getByText('Agent IA')).toBeInTheDocument();
      expect(screen.getByTestId('group-info-no-coach')).toHaveTextContent(
        "Ce groupe n'a pas encore de coach humain.",
      );
    });
  });

  it('withholds the report when the tenant tier does not enable the weekly digest', async () => {
    vi.mocked(groupsApi.getPermissions).mockResolvedValue({
      can_create: true,
      policy: 'everyone',
      weekly_digest: false,
    });
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole('button', { name: 'This week' }));
    expect(await screen.findByTestId('group-insights-tier-locked')).toBeInTheDocument();
    expect(screen.queryByTestId('group-report-summary')).toBeNull();
    expect(groupsApi.getWeeklyReport).not.toHaveBeenCalled();
    expect(groupsApi.getHealthFlags).not.toHaveBeenCalled();
  });

  it('offers the owner a quiet "Delete this group…", and archives it only from the confirm', async () => {
    const user = userEvent.setup();
    const { onMembershipEnded } = renderPanel();

    expect(screen.queryByTestId('group-info-leave')).toBeNull();
    const row = await screen.findByTestId('group-info-delete');
    expect(row).toHaveTextContent('Delete this group…');
    // No filled red anywhere in the drawer: the fill is the confirm's.
    expect(screen.getByTestId('group-info-panel').querySelector('.btn-danger')).toBeNull();
    await user.click(row);
    const confirm = await screen.findByRole('dialog');
    expect(confirm).toHaveTextContent('Delete this group?');
    const button = within(confirm).getByRole('button', { name: 'Delete group' });
    expect(button).toHaveClass('btn-danger');
    await user.click(button);

    await waitFor(() => expect(groupsApi.deleteGroup).toHaveBeenCalledWith(GROUP_ID));
    await waitFor(() => expect(onMembershipEnded).toHaveBeenCalledTimes(1));
  });

  it('offers a member a quiet "Leave this group…", and drops the thread once they are out', async () => {
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member', peer_sharing_consent: false })],
    });
    vi.mocked(groupsApi.leaveGroup).mockResolvedValue(undefined);
    const user = userEvent.setup();
    const { onMembershipEnded } = renderPanel();

    expect(await screen.findByTestId('group-info-leave')).toHaveTextContent('Leave this group…');
    expect(screen.queryByTestId('group-info-delete')).toBeNull();
    // A plain member sees no settings either, the digest select included.
    expect(screen.queryByRole('button', { name: /Group settings/ })).toBeNull();
    expect(screen.queryByTestId('group-digest-mode')).toBeNull();

    await user.click(screen.getByTestId('group-info-leave'));
    const confirm = await screen.findByRole('dialog');
    expect(confirm).toHaveTextContent('Leave this group?');
    await user.click(within(confirm).getByRole('button', { name: 'Leave group' }));

    await waitFor(() => expect(groupsApi.leaveGroup).toHaveBeenCalledWith(GROUP_ID));
    await waitFor(() => expect(onMembershipEnded).toHaveBeenCalledTimes(1));
  });

  it('never asks for the invite list on behalf of a plain member, which the route refuses', async () => {
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    renderPanel();

    expect(await screen.findByTestId('group-info-leave')).toBeInTheDocument();
    expect(screen.queryByText('Invites')).toBeNull();
    expect(groupsApi.listInvites).not.toHaveBeenCalled();
  });

  it('shows the group coach the TrainingPeaks section and none of the member-only surfaces', async () => {
    // The caller is the group's human coach and holds no membership row.
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Other' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({
      connections: [],
      total: 0,
      viewer: 'coach',
    });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        {
          provider_athlete_id: '900001',
          display_name: 'Alex Athlete',
          connection: null,
          suggested_member_user_id: OTHER_ID,
        },
      ],
    });
    const user = userEvent.setup();
    renderPanel();

    const section = await screen.findByTestId('delegation-section');
    expect(await within(section).findByTestId('delegation-roster-row-900001')).toHaveTextContent(
      'Alex Athlete',
    );
    expect(screen.getByTestId('group-info-coach-badge')).toHaveTextContent('You coach this group');
    expect(within(screen.getByTestId('group-info-agent')).getByText('Marathon Coach')).toBeInTheDocument();
    expect(screen.queryByTestId('group-info-leave')).toBeNull();
    expect(screen.queryByTestId('group-info-delete')).toBeNull();
    expect(screen.queryByTestId('peer-consent-card')).toBeNull();
    expect(screen.queryByTestId('coach-sharing-row')).toBeNull();
    expect(groupsApi.getCoachSharing).not.toHaveBeenCalled();
    // The record of a coach's reads is the athlete's, never the coach's.
    expect(screen.queryByTestId('coach-read-log')).toBeNull();
    expect(groupsApi.getCoachReads).not.toHaveBeenCalled();
    // The group's numbers belong to its members; the coach reads their triage on coach Home.
    expect(screen.queryByRole('button', { name: 'This week' })).toBeNull();
    // The coach sets where the weekly digest goes, and no other group setting.
    await user.click(screen.getByRole('button', { name: /Group settings/ }));
    expect(await screen.findByTestId('group-digest-mode')).toHaveValue('off');
    expect(screen.queryByLabelText('Group name')).toBeNull();
    expect(screen.queryByLabelText('Agent replies in the group chat')).toBeNull();
    expect(screen.queryByTestId('group-info-remove-coach')).toBeNull();
    // Stats and invites refuse a non-member, so the coach never asks for them.
    expect(groupsApi.getStats).not.toHaveBeenCalled();
    expect(groupsApi.listInvites).not.toHaveBeenCalled();
  });

  it("offers the group coach a way into each athlete's plan, in words, and opens that thread", async () => {
    // The caller is the group's human coach and holds no membership row.
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [
        member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Alice Runner' }),
        member({ id: 'membership-3', user_id: 'user-bob', role: 'member', display_name: 'Bob Cyclist' }),
      ],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    vi.mocked(chatApi.openAthleteThread).mockResolvedValue({
      id: 'conv-plan-alice',
      title: 'Alice Runner',
      subject_user_id: OTHER_ID,
      subject_group_id: GROUP_ID,
      subject_display_name: 'Alice Runner',
      message_count: 0,
      created_at: '2026-10-16T12:00:00Z',
      updated_at: '2026-10-16T12:00:00Z',
    });
    const user = userEvent.setup();
    const { onOpenConversation } = renderPanel();

    expect(await screen.findByRole('heading', { name: 'Your athletes' })).toBeInTheDocument();
    // The visible label leads the name, the athlete follows it (WCAG 2.5.3).
    const bob = await screen.findByRole('button', { name: 'Open their plan thread · Bob Cyclist' });
    // With two athletes, neither card claims the drawer's one filled button.
    expect(bob).toHaveClass('btn-outline');
    await user.click(screen.getByRole('button', { name: 'Open their plan thread · Alice Runner' }));

    await waitFor(() => expect(onOpenConversation).toHaveBeenCalledWith('conv-plan-alice'));
    expect(chatApi.openAthleteThread).toHaveBeenCalledWith(OTHER_ID, GROUP_ID);
  });

  it("makes a pilot coach's one athlete's plan thread the drawer's one filled button", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Alice Runner' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    renderPanel();

    const open = await screen.findByTestId(`member-work-on-plan-${OTHER_ID}`);
    expect(open).toHaveClass('btn-primary');
    expect(screen.getByTestId('group-info-panel').querySelectorAll('.btn-primary')).toHaveLength(1);
    expect(screen.getByTestId(`member-view-plan-${OTHER_ID}`)).toHaveTextContent('View plan');
  });

  it('opens the athlete on coach Home for a coach who has one, and offers it to no one else', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Alice Runner' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    authUser.manages_roster = true;
    const user = userEvent.setup();
    const { onOpenCoachHome, unmount } = renderPanel();

    await user.click(await screen.findByRole('button', { name: 'See on coach Home · Alice Runner' }));
    expect(onOpenCoachHome).toHaveBeenCalledWith(OTHER_ID);
    unmount();

    // A coach without coach Home has nowhere to go.
    authUser.manages_roster = undefined;
    renderPanel();
    await screen.findByTestId(`member-work-on-plan-${OTHER_ID}`);
    expect(screen.queryByTestId(`member-coach-home-${OTHER_ID}`)).toBeNull();
  });

  it("offers a coach who is also a member every other member's plan, never their own", async () => {
    // The caller owns the group, holds its membership row, and coaches it:
    // the server admits them for every other member and refuses them for
    // themselves.
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: CALLER_ID }));
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    renderPanel();

    expect(await screen.findByRole('button', { name: 'Open their plan thread · Other' })).toBeInTheDocument();
    expect(screen.getByTestId(`member-work-on-plan-${OTHER_ID}`)).toBeInTheDocument();
    expect(screen.queryByTestId(`member-work-on-plan-${CALLER_ID}`)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Open their plan thread · Caller' })).toBeNull();
  });

  describe("reading a member's live plan, read-only (carnet#1244)", () => {
    const PLAN: WorkoutPlan = {
      goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
      phases: [],
      weeks: [],
      weeks_deferred: 0,
    };

    function coachOf(display_name = 'Alice Runner') {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
      );
      vi.mocked(groupsApi.listMembers).mockResolvedValue({
        members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name })],
      });
      vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
      vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    }

    it('shows the coach the plan the athlete sees, and works on it from there', async () => {
      coachOf();
      vi.mocked(chatApi.getAthletePlan).mockResolvedValue({
        athlete: { user_id: OTHER_ID, display_name: 'Alice Runner' },
        today: '2026-10-28',
        live: PLAN,
      });
      vi.mocked(chatApi.openAthleteThread).mockResolvedValue({
        id: 'conv-plan-alice',
        title: 'Alice Runner',
        message_count: 0,
        created_at: '2026-10-16T12:00:00Z',
        updated_at: '2026-10-16T12:00:00Z',
      });
      const user = userEvent.setup();
      const { onOpenConversation } = renderPanel();

      await user.click(await screen.findByRole('button', { name: 'View plan · Alice Runner' }));

      expect(await screen.findByText('Read-only: the plan Alice Runner sees.')).toBeInTheDocument();
      expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
      expect(chatApi.getAthletePlan).toHaveBeenCalledWith(OTHER_ID, GROUP_ID);
      expect(chatApi.openAthleteThread).not.toHaveBeenCalled();

      await user.click(screen.getByTestId('athlete-plan-work-on'));
      await waitFor(() => expect(onOpenConversation).toHaveBeenCalledWith('conv-plan-alice'));
    });

    it('says the athlete has no published plan', async () => {
      coachOf();
      vi.mocked(chatApi.getAthletePlan).mockResolvedValue({
        athlete: { user_id: OTHER_ID, display_name: 'Alice Runner' },
        today: '2026-10-28',
        live: null,
      });
      const user = userEvent.setup();
      renderPanel();

      await user.click(await screen.findByRole('button', { name: 'View plan · Alice Runner' }));

      expect(await screen.findByText('Alice Runner has no published plan yet.')).toBeInTheDocument();
    });

    it('says the athlete stopped sharing, and offers no way into the thread', async () => {
      coachOf();
      vi.mocked(chatApi.getAthletePlan).mockRejectedValue(
        Object.assign(new Error('Request failed with status code 403'), {
          response: { status: 403, data: { code: 'PermissionDenied', details: { reason: 'coach_sharing_revoked' } } },
        }),
      );
      const user = userEvent.setup();
      renderPanel();

      await user.click(await screen.findByRole('button', { name: 'View plan · Alice Runner' }));

      expect(
        await screen.findByText("Alice Runner stopped sharing with you, so their plan isn't shown."),
      ).toBeInTheDocument();
      expect(screen.queryByTestId('athlete-plan-work-on')).toBeNull();
    });

    it('never offers a member the view', async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID }));
      renderPanel();

      expect(await screen.findByText('Other')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /View plan/ })).toBeNull();
      expect(chatApi.getAthletePlan).not.toHaveBeenCalled();
    });
  });

  describe("reading and setting a member's thresholds (carnet#1561)", () => {
    const FTP = {
      anchor: 'ftp' as const,
      unit: 'watts' as const,
      value: 285,
      low: null,
      high: null,
      origin: 'athlete',
      as_of: '2026-10-20',
      method: null,
      confidence: 'medium' as const,
      value_text: '285 W',
      range_text: null,
      source_text: "the athlete's own value",
    };
    const OFFERED = [{ sport: 'ride' as const, anchors: [{ anchor: 'ftp' as const, unit: 'watts' as const }] }];

    function coachOf() {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
      );
      vi.mocked(groupsApi.listMembers).mockResolvedValue({
        members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Alice Runner' })],
      });
      vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
      vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    }

    it("shows the coach the athlete's thresholds worded for a coach, and saves one as theirs to set", async () => {
      coachOf();
      vi.mocked(chatApi.getAthletePhysiology).mockResolvedValue({
        sports: [{ sport: 'ride', anchors: [FTP] }],
        offered: OFFERED,
      });
      vi.mocked(chatApi.updateAthletePhysiologyAnchor).mockResolvedValue({
        sports: [
          {
            sport: 'ride',
            anchors: [{ ...FTP, value: 292, value_text: '292 W', origin: 'coach', source_text: 'set by a coach' }],
          },
        ],
        offered: OFFERED,
      });
      const user = userEvent.setup();
      renderPanel();

      await user.click(await screen.findByRole('button', { name: 'Thresholds · Alice Runner' }));

      expect(await screen.findByText("Alice Runner's thresholds")).toBeInTheDocument();
      expect(screen.getByTestId('physiology-ride-ftp-source')).toHaveTextContent("the athlete's own value");
      expect(chatApi.getAthletePhysiology).toHaveBeenCalledWith(OTHER_ID, GROUP_ID, 'en');

      await user.click(screen.getByTestId('physiology-ride-ftp-edit'));
      const editor = await screen.findByTestId('physiology-editor');
      const value = within(editor).getByTestId('physiology-editor-value');
      await user.clear(value);
      await user.type(value, '292');
      await user.selectOptions(within(editor).getByTestId('physiology-editor-kind'), 'measured');
      await user.click(within(editor).getByTestId('physiology-editor-save'));

      await waitFor(() =>
        expect(chatApi.updateAthletePhysiologyAnchor).toHaveBeenCalledWith(
          OTHER_ID,
          GROUP_ID,
          'ride',
          'ftp',
          expect.objectContaining({ value: 292, kind: 'measured', confidence: 'medium', pace_per: null }),
          'en',
        ),
      );
      expect(await screen.findByTestId('athlete-physiology-saved')).toHaveTextContent('Saved. Alice Runner is told.');
      expect(screen.getByTestId('physiology-ride-ftp-source')).toHaveTextContent('set by a coach');
    });

    it('says the athlete stopped sharing, and offers nothing to set', async () => {
      coachOf();
      vi.mocked(chatApi.getAthletePhysiology).mockRejectedValue(
        Object.assign(new Error('Request failed with status code 403'), {
          response: { status: 403, data: { code: 'PermissionDenied', details: { reason: 'coach_sharing_revoked' } } },
        }),
      );
      const user = userEvent.setup();
      renderPanel();

      await user.click(await screen.findByRole('button', { name: 'Thresholds · Alice Runner' }));

      expect(await screen.findByTestId('athlete-physiology-revoked')).toHaveTextContent(
        'Alice Runner stopped sharing their data with you, so their thresholds are hidden.',
      );
      expect(screen.queryByTestId('athlete-physiology-add')).toBeNull();
    });

    it('never offers a member the thresholds of another', async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID }));
      renderPanel();

      expect(await screen.findByText('Other')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /Thresholds/ })).toBeNull();
      expect(chatApi.getAthletePhysiology).not.toHaveBeenCalled();
    });
  });

  describe('when the server refuses the plan thread', () => {
    beforeEach(() => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
      );
      vi.mocked(groupsApi.listMembers).mockResolvedValue({
        members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Alice Runner' })],
      });
      vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
      vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    });

    it('says the athlete stopped sharing, which no retry changes, and stays put', async () => {
      // The coaching gate refuses with its reason in `details`, as every
      // platform refusal carries it.
      vi.mocked(chatApi.openAthleteThread).mockRejectedValue(
        Object.assign(new Error('Request failed with status code 403'), {
          response: { status: 403, data: { code: 'PermissionDenied', details: { reason: 'coach_sharing_revoked' } } },
        }),
      );
      const user = userEvent.setup();
      const { onOpenConversation } = renderPanel();

      await user.click(await screen.findByRole('button', { name: 'Open their plan thread · Alice Runner' }));

      expect(
        await screen.findByText("Alice Runner stopped sharing with you, so their plan can't be changed."),
      ).toBeInTheDocument();
      expect(screen.queryByText("We couldn't open this plan thread. Please try again.")).toBeNull();
      expect(onOpenConversation).not.toHaveBeenCalled();
    });

    it('asks the coach to try again on any other refusal, and stays put', async () => {
      vi.mocked(chatApi.openAthleteThread).mockRejectedValue(
        Object.assign(new Error('Request failed with status code 404'), {
          response: { status: 404, data: { code: 'ResourceNotFound', details: { reason: 'not_a_member' } } },
        }),
      );
      const user = userEvent.setup();
      const { onOpenConversation } = renderPanel();

      await user.click(await screen.findByRole('button', { name: 'Open their plan thread · Alice Runner' }));

      expect(await screen.findByText("We couldn't open this plan thread. Please try again.")).toBeInTheDocument();
      expect(onOpenConversation).not.toHaveBeenCalled();
    });
  });

  it("never offers a member, even the owner, a way into another member's plan", async () => {
    // The default caller owns the group and is a member of it; a coach is
    // attached, but it is not them.
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID }));
    renderPanel();

    expect(await screen.findByText('Other')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Open their plan thread/ })).toBeNull();
    expect(chatApi.openAthleteThread).not.toHaveBeenCalled();
  });

  it("never offers a plain member a way into anyone's plan", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID, coach_user_id: COACH_ID }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [
        member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Other' }),
        member({ role: 'member' }),
      ],
    });
    renderPanel();

    expect(await screen.findByText('Other')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Open their plan thread/ })).toBeNull();
  });

  it('draws no filled red for an athlete either: every exit and removal is quiet until its confirm', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ owner_id: OTHER_ID, coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'admin' }), member({ id: 'membership-2', user_id: OTHER_ID, role: 'member', display_name: 'Other' })],
    });
    renderPanel();

    await screen.findByTestId('group-info-leave');
    expect(screen.getByTestId('group-info-panel').querySelector('.btn-danger')).toBeNull();
  });

  it('shows a coach who is also a member the TrainingPeaks section the server names them coach of', async () => {
    // The caller owns the group, holds its membership row, and coaches it.
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: CALLER_ID }));
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({
      connections: [],
      total: 0,
      viewer: 'coach',
    });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        {
          provider_athlete_id: '900001',
          display_name: 'Alex Athlete',
          connection: null,
          suggested_member_user_id: null,
        },
      ],
    });

    renderPanel();

    const section = await screen.findByTestId('delegation-section');
    expect(await within(section).findByTestId('delegation-roster-row-900001')).toHaveTextContent(
      'Alex Athlete',
    );
    expect(screen.getByTestId('group-info-coach-badge')).toHaveTextContent('You coach this group');
    // A member still: their own surfaces stay.
    expect(screen.getByTestId('peer-consent-card')).toBeInTheDocument();
  });

  it('asks a member to confirm the link their coach proposed, and confirms it', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({
      connections: [link()],
      total: 1,
      viewer: 'member',
    });
    vi.mocked(groupsApi.confirmDelegatedConnection).mockResolvedValue(
      link({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z' }),
    );
    const user = userEvent.setup();
    renderPanel();

    const request = await screen.findByTestId('delegation-request');
    expect(request).toHaveTextContent(
      "Casey Coach coaches you on TrainingPeaks as Alex Athlete. Confirm, and Dravr reads your TrainingPeaks workouts through Casey Coach's account",
    );
    expect(screen.queryByTestId('delegation-section')).toBeNull();

    await user.click(within(request).getByTestId('delegation-confirm'));
    await waitFor(() =>
      // A TrainingPeaks link offers no consent, so the confirm carries no body.
      expect(groupsApi.confirmDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, 'dc-1', undefined),
    );
  });

  it('warns a linked member that leaving ends the link too', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({
      connections: [link({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z' })],
      total: 1,
      viewer: 'member',
    });
    const user = userEvent.setup();
    renderPanel();

    expect(await screen.findByTestId('delegation-linked')).toHaveTextContent(
      "Your TrainingPeaks workouts are read through Casey Coach's account.",
    );
    await user.click(screen.getByTestId('group-info-leave'));
    const confirm = await screen.findByRole('dialog');
    expect(confirm).toHaveTextContent('Your TrainingPeaks link through this group ends too.');
  });

  it("shows every live link of a member with its own platform, and one coach-consents block for them all", async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    // The server lists the newest proposal first: a TrainingPeaks link the
    // coach just proposed, above the Intervals.icu link the member confirmed
    // earlier with their wellness shared.
    const intervals = link({
      id: 'dc-intervals',
      provider: 'intervals_icu',
      provider_athlete_id: 'i201',
      status: 'confirmed',
      proposed_at: '2026-09-20T08:00:00Z',
      confirmed_at: '2026-09-20T09:00:00Z',
      calendar_write_supported: true,
      wellness_read: true,
      wellness_read_supported: true,
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({
      connections: [link({ id: 'dc-trainingpeaks', proposed_at: '2026-10-14T08:00:00Z' }), intervals],
      total: 2,
      viewer: 'member',
    });
    const held = coachConsents({ calendar_write_supported: true, wellness_read: true, wellness_read_supported: true });
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(held);
    vi.mocked(groupsApi.updateCoachConsents).mockResolvedValue({ ...held, wellness_read: false });
    const user = userEvent.setup();
    renderPanel();

    expect(await screen.findByText('TrainingPeaks through your human coach')).toBeInTheDocument();
    expect(screen.getByText('Intervals.icu through your human coach')).toBeInTheDocument();
    const proposal = await screen.findByTestId('delegation-member-link-dc-trainingpeaks');
    expect(within(proposal).getByTestId('delegation-request')).toHaveTextContent('TrainingPeaks link request');

    const confirmed = screen.getByTestId('delegation-member-link-dc-intervals');
    expect(within(confirmed).getByTestId('delegation-linked')).toHaveTextContent(
      "Your Intervals.icu workouts are read through Casey Coach's account.",
    );
    // The consents live on the coach relationship: one block, none on the link.
    expect(within(confirmed).queryByRole('checkbox')).toBeNull();
    expect(screen.getByText('What your human coach can do')).toBeInTheDocument();
    const wellness = await screen.findByTestId('coach-consent-wellness_read');
    expect(wellness).toBeChecked();
    await user.click(wellness);
    await user.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Stop sharing' }));
    await waitFor(() =>
      expect(groupsApi.updateCoachConsents).toHaveBeenCalledWith(GROUP_ID, { wellness_read: false }),
    );

    // Leaving ends both, and the warning names both.
    await user.click(screen.getByTestId('group-info-leave'));
    const leave = await screen.findByRole('dialog');
    expect(leave).toHaveTextContent('Your TrainingPeaks link through this group ends too.');
    expect(leave).toHaveTextContent('Your Intervals.icu link through this group ends too.');
  });

  it('lets a member with no link allow calendar writes, and asks them for their wellness too', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'member' });
    // Every coached member is offered wellness, which covers each app they connect.
    const offered = { calendar_write_supported: true, wellness_read_supported: true };
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(coachConsents(offered));
    vi.mocked(groupsApi.updateCoachConsents).mockResolvedValue(coachConsents({ ...offered, calendar_write: true }));
    const user = userEvent.setup();
    renderPanel();

    const calendar = await screen.findByTestId('coach-consent-calendar_write');
    expect(screen.queryByTestId('delegation-linked')).toBeNull();
    expect(screen.getByTestId('coach-consent-calendar-step')).toHaveTextContent(
      'open intervals.icu/athletes, click Casey Coach and pick the Coach role, not Follower.',
    );
    expect(screen.getByTestId('coach-consent-wellness_read')).not.toBeChecked();
    await user.click(calendar);
    await waitFor(() =>
      expect(groupsApi.updateCoachConsents).toHaveBeenCalledWith(GROUP_ID, { calendar_write: true }),
    );
  });

  it('never asks a member of a coachless group for coach consents', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup());
    renderPanel();
    await screen.findByText('Marathon Squad');
    expect(groupsApi.getCoachConsents).not.toHaveBeenCalled();
    expect(screen.queryByTestId('coach-consents')).toBeNull();
  });

  describe('the chat app the room is connected to (carnet#1683)', () => {
    const en = i18n.getFixedT('en');

    afterEach(() => {
      vi.useRealTimers();
    });

    /** Answer the connection read from a flag the test flips, as the server would after a write. */
    function connectionFrom(state: { connected: boolean }, canManage: boolean) {
      vi.mocked(groupsApi.getChatConnection).mockImplementation(async () =>
        chatConnection(state.connected ? { ...CONNECTED, can_manage: canManage } : { can_manage: canManage }),
      );
    }

    async function startConnecting(user: ReturnType<typeof userEvent.setup>) {
      await user.click(await screen.findByTestId('group-chat-connect'));
      await user.click(screen.getByTestId('group-chat-choose-telegram'));
    }

    it('offers the owner a way to connect a chat app: Telegram now, WhatsApp shown as coming', async () => {
      vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection({ can_manage: true }));
      const user = userEvent.setup();
      renderPanel();

      const section = await screen.findByTestId('group-chat-connection');
      expect(within(section).getByTestId('group-chat-not-connected')).toHaveTextContent(
        en('humanCoach.groupChat.web.notConnected'),
      );
      await user.click(within(section).getByTestId('group-chat-connect'));
      expect(within(section).getByTestId('group-chat-choose-telegram')).toBeEnabled();
      const whatsapp = within(section).getByTestId('group-chat-choose-whatsapp');
      expect(whatsapp).toBeDisabled();
      expect(whatsapp).toHaveTextContent(en('humanCoach.groupChat.web.comingSoon'));
      // Choosing is not connecting: no code is made until Telegram is picked.
      expect(groupsApi.createChatConnection).not.toHaveBeenCalled();
    });

    it('hands the manager the deep link, the command to type and the expiry, without a QR on a narrow screen', async () => {
      vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection({ can_manage: true }));
      const code = connectCode();
      vi.mocked(groupsApi.createChatConnection).mockResolvedValue(code);
      const user = userEvent.setup();
      renderPanel();

      await startConnecting(user);

      const pending = await screen.findByTestId('group-chat-pending');
      expect(groupsApi.createChatConnection).toHaveBeenCalledWith(GROUP_ID, { channel: 'telegram' });
      const open = within(pending).getByTestId('group-chat-open-app');
      expect(open).toHaveAttribute('href', 'https://t.me/dravr_bot?startgroup=K7Q2ZP');
      expect(open).toHaveTextContent('Open Telegram');
      expect(within(pending).getByTestId('group-chat-command')).toHaveTextContent('/group link K7Q2ZP');
      expect(within(pending).getByTestId('group-chat-expiry')).toHaveTextContent(
        en('humanCoach.groupChat.web.expiresAt', { time: formatMessageTime(code.expires_at, 'en') }),
      );
      expect(within(pending).queryByTestId('group-chat-qr')).toBeNull();

      await user.click(within(pending).getByTestId('group-chat-copy-command'));
      expect(await navigator.clipboard.readText()).toBe('/group link K7Q2ZP');
    });

    it('adds the QR on a wide screen, where the chat app runs on another device', async () => {
      const wide = vi.spyOn(window, 'matchMedia').mockImplementation(
        (query: string) =>
          ({
            matches: query === '(min-width: 1024px)',
            media: query,
            onchange: null,
            addEventListener: () => {},
            removeEventListener: () => {},
            addListener: () => {},
            removeListener: () => {},
            dispatchEvent: () => false,
          }) as MediaQueryList,
      );
      try {
        vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection({ can_manage: true }));
        vi.mocked(groupsApi.createChatConnection).mockResolvedValue(connectCode());
        const user = userEvent.setup();
        renderPanel();

        await startConnecting(user);

        const qr = await screen.findByTestId('group-chat-qr');
        expect(within(qr).getByRole('img')).toHaveAttribute(
          'src',
          `data:image/svg+xml;utf8,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg"></svg>')}`,
        );
      } finally {
        wide.mockRestore();
      }
    });

    it('reads the connection again while the code is out and turns to the chat once the bot joins', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const state = { connected: false };
      connectionFrom(state, true);
      vi.mocked(groupsApi.createChatConnection).mockResolvedValue(connectCode());
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
      renderPanel();

      await startConnecting(user);
      await screen.findByTestId('group-chat-pending');
      const readsBefore = vi.mocked(groupsApi.getChatConnection).mock.calls.length;

      // The bot joins on the manager's phone; only the next read can tell this screen.
      state.connected = true;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3_000);
      });

      const row = await screen.findByTestId('group-chat-connected');
      expect(vi.mocked(groupsApi.getChatConnection).mock.calls.length).toBeGreaterThan(readsBefore);
      expect(within(row).getByTestId('group-chat-title')).toHaveTextContent('Sunday Riders');
      expect(screen.queryByTestId('group-chat-pending')).toBeNull();
    });

    it('says a code expired and makes a new one on request', async () => {
      vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection({ can_manage: true }));
      vi.mocked(groupsApi.createChatConnection)
        .mockResolvedValueOnce(connectCode({ expires_at: new Date(Date.now() - 1_000).toISOString() }))
        .mockResolvedValueOnce(connectCode({ code: 'M4X9TB', command: '/group link M4X9TB' }));
      const user = userEvent.setup();
      renderPanel();

      await startConnecting(user);

      const expired = await screen.findByTestId('group-chat-expired');
      expect(expired).toHaveTextContent(en('humanCoach.groupChat.web.expired'));
      expect(screen.queryByTestId('group-chat-open-app')).toBeNull();
      await user.click(within(expired).getByTestId('group-chat-new-code'));
      expect(await screen.findByTestId('group-chat-command')).toHaveTextContent('/group link M4X9TB');
      expect(groupsApi.createChatConnection).toHaveBeenCalledTimes(2);
    });

    it('shows the chat a peer connected meanwhile when making a code is refused as already connected', async () => {
      const state = { connected: false };
      connectionFrom(state, true);
      vi.mocked(groupsApi.createChatConnection).mockImplementation(async () => {
        state.connected = true;
        throw { response: { status: 409, data: { message: 'The group is already connected to a chat' } } };
      });
      const user = userEvent.setup();
      renderPanel();

      await startConnecting(user);

      expect(await screen.findByTestId('group-chat-connected')).toHaveTextContent('Sunday Riders');
      expect(await screen.findByText(en('humanCoach.groupChat.web.alreadyConnected'))).toBeInTheDocument();
      expect(screen.queryByTestId('group-chat-pending')).toBeNull();
    });

    it('shows the manager the connected chat and disconnects it only after a confirm', async () => {
      const state = { connected: true };
      connectionFrom(state, true);
      vi.mocked(groupsApi.deleteChatConnection).mockImplementation(async () => {
        state.connected = false;
      });
      const user = userEvent.setup();
      renderPanel();

      const row = await screen.findByTestId('group-chat-connected');
      expect(within(row).getByTestId('group-chat-title')).toHaveTextContent('Sunday Riders');
      expect(within(row).getByTestId('group-chat-connected-to')).toHaveTextContent(
        en('humanCoach.groupChat.web.connectedTo', { app: 'Telegram' }),
      );
      await user.click(within(row).getByTestId('group-chat-disconnect'));
      expect(groupsApi.deleteChatConnection).not.toHaveBeenCalled();

      const confirm = await screen.findByRole('dialog');
      await user.click(within(confirm).getByRole('button', { name: 'Disconnect' }));
      await waitFor(() => expect(groupsApi.deleteChatConnection).toHaveBeenCalledWith(GROUP_ID));
      // Read again after the write: the owner is offered the connect again.
      expect(await screen.findByTestId('group-chat-not-connected')).toBeInTheDocument();
      expect(screen.queryByTestId('group-chat-connected')).toBeNull();
    });

    it("offers the group's coach the connect when the server says they manage it, member or not", async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(
        sampleGroup({ coach_user_id: CALLER_ID, coach_display_name: 'Caller Coach', owner_id: OTHER_ID }),
      );
      vi.mocked(groupsApi.listMembers).mockResolvedValue({
        members: [member({ id: 'membership-2', user_id: OTHER_ID, role: 'owner', display_name: 'Alice Runner' })],
      });
      vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
      vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
      vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection({ can_manage: true }));
      renderPanel();

      expect(await screen.findByTestId('group-chat-connect')).toBeInTheDocument();
    });

    it('shows a member the connected chat read-only, with no way to disconnect or connect it', async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
      vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
      vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection(CONNECTED));
      renderPanel();

      const row = await screen.findByTestId('group-chat-connected');
      expect(within(row).getByTestId('group-chat-title')).toHaveTextContent('Sunday Riders');
      expect(within(row).getByTestId('group-chat-connected-to')).toHaveTextContent(
        en('humanCoach.groupChat.web.connectedTo', { app: 'Telegram' }),
      );
      expect(screen.queryByTestId('group-chat-disconnect')).toBeNull();
      expect(screen.queryByTestId('group-chat-connect')).toBeNull();
    });

    it('names an untitled chat by its app', async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
      vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
      vi.mocked(groupsApi.getChatConnection).mockResolvedValue(chatConnection({ ...CONNECTED, chat_title: null }));
      renderPanel();

      expect(await screen.findByTestId('group-chat-title')).toHaveTextContent(
        en('humanCoach.groupChat.web.untitledChat', { app: 'Telegram' }),
      );
    });

    it('shows a member nothing while no chat is connected', async () => {
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
      vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
      renderPanel();

      await screen.findByTestId('group-info-more');
      await waitFor(() => expect(groupsApi.getChatConnection).toHaveBeenCalledWith(GROUP_ID));
      await act(async () => {});
      expect(screen.queryByTestId('group-chat-connection')).toBeNull();
    });
  });
});
