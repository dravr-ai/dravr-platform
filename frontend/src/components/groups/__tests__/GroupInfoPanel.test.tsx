// ABOUTME: Tests for Group info inside chat — who runs the group, consent, roster, settings, the digest, the exits, the coach's view
// ABOUTME: Carries over the GroupDetail cases: the caller's OWN consent row, and the tier-gated report
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { cleanup, render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import GroupInfoPanel from '../GroupInfoPanel';
import { ToastProvider } from '../../ui';
import type { CoachConsentsResponse, CoachingGroup, DelegatedConnection, GroupMember } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import { UnitsContext } from '@pierre/ui-logic';
import type { DistanceUnit } from '@pierre/chat-utils';

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
    leaveGroup: vi.fn(),
    deleteGroup: vi.fn(),
    removeCoach: vi.fn(),
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
  },
  providersApi: {
    getProvidersStatus: vi.fn(),
  },
  userApi: {
    getOnboardingStatus: vi.fn(),
    getCoachAccessRequest: vi.fn(),
    requestCoachAccess: vi.fn(),
  },
  chatApi: {
    openAthleteThread: vi.fn(),
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

const { chatApi, groupsApi, userApi } = await import('../../../services/api');

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
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const utils = render(
    <QueryClientProvider client={queryClient}>
      <UnitsContext.Provider value={units}>
        <ToastProvider>
          <GroupInfoPanel
            groupId={GROUP_ID}
            onMembershipEnded={onMembershipEnded}
            onOpenConversation={onOpenConversation}
          />
        </ToastProvider>
      </UnitsContext.Provider>
    </QueryClientProvider>,
  );
  return { ...utils, onMembershipEnded, onOpenConversation };
}

describe('GroupInfoPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    authUser.manages_roster = undefined;
    vi.mocked(userApi.getOnboardingStatus).mockResolvedValue(onboardingStatus(false));
    vi.mocked(userApi.getCoachAccessRequest).mockResolvedValue({ request: null });
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
        total_members: 2,
        active_members: 2,
        avg_weekly_volume_km: 41.5,
        avg_ctl: 55,
        flagged_members: 0,
        weekly_trend: 'stable',
      },
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
    });
    vi.mocked(groupsApi.updatePeerConsent).mockResolvedValue(undefined);
    vi.mocked(groupsApi.updateGroup).mockResolvedValue(sampleGroup());
    vi.mocked(groupsApi.deleteGroup).mockResolvedValue(undefined);
    vi.mocked(groupsApi.listInvites).mockResolvedValue({ invites: [] });
    vi.mocked(groupsApi.getTranscript).mockResolvedValue({ entries: [], total: 0 });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({
      connections: [],
      total: 0,
      viewer: 'member',
    });
  });

  it('names the group, its description and its roster size', async () => {
    renderPanel();

    expect(await screen.findByTestId('group-info-name')).toHaveTextContent('Marathon Squad');
    expect(screen.getByTestId('group-info-description')).toHaveTextContent('Sunday long runs');
    expect(await screen.findByText('Members: 2')).toBeInTheDocument();
  });

  it('names the AI agent and the human coach above the members for a plain member', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ owner_id: OTHER_ID, coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
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
    // Both rows lead the roster: they sit before the member table.
    const table = await screen.findByRole('table');
    expect(agent.compareDocumentPosition(table) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(coach.compareDocumentPosition(table) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('says the group has no human coach yet when none is attached', async () => {
    renderPanel();

    expect(await screen.findByTestId('group-info-no-coach')).toHaveTextContent(
      'No human coach in this group yet.',
    );
    expect(screen.queryByTestId('group-info-coach-row')).toBeNull();
    expect(screen.queryByTestId('group-info-coach-badge')).toBeNull();
  });

  describe('coach access request (carnet#738)', () => {
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

    it('offers the owner who coaches others a one-tap request naming this group', async () => {
      vi.mocked(userApi.getOnboardingStatus).mockResolvedValue(onboardingStatus(true));
      vi.mocked(userApi.requestCoachAccess).mockResolvedValue({ request: pending });
      renderPanel();

      const row = await screen.findByTestId('group-info-no-coach');
      const button = await within(row).findByTestId('coach-access-request');
      await userEvent.click(button);

      await waitFor(() => expect(userApi.requestCoachAccess).toHaveBeenCalledWith(GROUP_ID));
      expect(await within(row).findByTestId('coach-access-pending')).toBeInTheDocument();
      expect(within(row).queryByTestId('coach-access-request')).toBeNull();
    });

    it('shows a request already waiting instead of the button', async () => {
      vi.mocked(userApi.getOnboardingStatus).mockResolvedValue(onboardingStatus(true));
      vi.mocked(userApi.getCoachAccessRequest).mockResolvedValue({ request: pending });
      renderPanel();

      expect(await screen.findByTestId('coach-access-pending')).toBeInTheDocument();
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
    });

    it('offers nothing to an owner who does not coach others', async () => {
      renderPanel();

      await screen.findByTestId('group-info-no-coach');
      await waitFor(() => expect(userApi.getOnboardingStatus).toHaveBeenCalled());
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
    });

    it('offers nothing to an owner who already holds coach access', async () => {
      authUser.manages_roster = true;
      vi.mocked(userApi.getOnboardingStatus).mockResolvedValue(onboardingStatus(true));
      renderPanel();

      await screen.findByTestId('group-info-no-coach');
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
      expect(userApi.getOnboardingStatus).not.toHaveBeenCalled();
    });

    it('offers nothing to a plain member of a coachless group', async () => {
      vi.mocked(userApi.getOnboardingStatus).mockResolvedValue(onboardingStatus(true));
      vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ owner_id: OTHER_ID }));
      vi.mocked(groupsApi.listMembers).mockResolvedValue({ members: [member({ role: 'member' })] });
      renderPanel();

      await screen.findByTestId('group-info-no-coach');
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
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

  it('names the coach in the admin coach section and never prints their id', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ coach_user_id: COACH_UUID, coach_display_name: 'Casey Coach' }),
    );
    renderPanel();

    expect(await screen.findByTestId('group-info-coach-name')).toHaveTextContent('Casey Coach');
    expect(screen.getByTestId('group-info-remove-coach')).toBeInTheDocument();
    expect(screen.getByTestId('group-info-panel').textContent).not.toContain(COACH_UUID);
    expect(document.body.textContent).not.toContain(COACH_UUID);
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

  it('lists the roster with each member role', async () => {
    renderPanel();

    const roster = within(await screen.findByRole('table'));
    expect(roster.getByText('Caller')).toBeInTheDocument();
    expect(roster.getByText('Other')).toBeInTheDocument();
    expect(roster.getByText('Owner')).toBeInTheDocument();
    expect(roster.getByText('Member')).toBeInTheDocument();
  });

  it('saves the group settings an admin edits', async () => {
    const user = userEvent.setup();
    renderPanel();

    const name = await screen.findByLabelText('Group Name');
    await user.clear(name);
    await user.type(name, 'Harricana Squad');
    await user.click(screen.getByTestId('group-info-save-settings'));

    await waitFor(() =>
      expect(groupsApi.updateGroup).toHaveBeenCalledWith(GROUP_ID, {
        name: 'Harricana Squad',
        description: 'Sunday long runs',
        peer_data_sharing: true,
        respond_mode: 'all',
        digest_mode: 'off',
      }),
    );
  });

  it('lets an admin choose where the weekly digest goes, with the chosen mode explained', async () => {
    const user = userEvent.setup();
    renderPanel();

    const select = await screen.findByTestId('group-digest-mode');
    expect(select).toHaveValue('off');
    expect(within(select).getAllByRole('option').map((o) => o.textContent)).toEqual([
      'Off',
      'In the group chat',
      'Owner and admins only',
    ]);
    expect(screen.getByText('Nothing is sent.')).toBeInTheDocument();

    await user.selectOptions(select, 'managers');
    expect(
      screen.getByText(
        "The owner and admins get the full recap on their own channels; nothing is posted in the group's chat.",
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByTestId('group-info-save-settings'));

    await waitFor(() =>
      expect(groupsApi.updateGroup).toHaveBeenCalledWith(GROUP_ID, {
        name: 'Marathon Squad',
        description: 'Sunday long runs',
        peer_data_sharing: true,
        respond_mode: 'all',
        digest_mode: 'managers',
      }),
    );
  });

  it('gives the attached coach the digest select alone and sends only the digest mode', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(
      sampleGroup({ owner_id: OTHER_ID, coach_user_id: CALLER_ID, digest_mode: 'chat' }),
    );
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    const user = userEvent.setup();
    renderPanel();

    const select = await screen.findByTestId('group-digest-mode');
    expect(select).toHaveValue('chat');
    // The coach changes the digest and nothing else: no name, no respond mode.
    expect(screen.queryByLabelText('Group Name')).toBeNull();
    expect(screen.queryByLabelText('Agent replies in the group chat')).toBeNull();

    await user.selectOptions(select, 'off');
    await user.click(screen.getByTestId('group-info-save-settings'));

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

    await screen.findByTestId('group-insights-tier-locked');
    expect(screen.queryByTestId('group-digest-mode')).toBeNull();
    await user.click(screen.getByTestId('group-info-save-settings'));

    await waitFor(() =>
      expect(groupsApi.updateGroup).toHaveBeenCalledWith(GROUP_ID, {
        name: 'Marathon Squad',
        description: 'Sunday long runs',
        peer_data_sharing: true,
        respond_mode: 'all',
      }),
    );
  });

  it('renders the weekly report and one health-flag row per flagged member', async () => {
    renderPanel();

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
    const recommendations = screen.getAllByTestId('group-report-recommendation');
    expect(recommendations.map((r) => r.textContent)).toEqual([
      'Members at high overtraining risk: 1. Consider adjusting their recovery.',
      'Group volume is steady compared with last week.',
    ]);
    const rows = screen.getAllByTestId('group-health-flag-row');
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText('No activity for 11 days')).toBeInTheDocument();
    expect(screen.getByText('Health flags (2)')).toBeInTheDocument();
    expect(within(screen.getByTestId('group-info-stats')).getByText('of 2 total')).toBeInTheDocument();
  });

  it('prints the group volume in miles for a reader on imperial units', async () => {
    renderPanel('imperial');

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
      renderPanel();

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
      expect(stats.getByText('sur 2 au total')).toBeInTheDocument();
      // The panel's own chrome reads French too: the member count, the
      // caller's marker and the join dates.
      expect(screen.getByText('Membres : 2')).toBeInTheDocument();
      expect(screen.getByText('(toi)')).toBeInTheDocument();
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

    renderPanel();

    expect(await screen.findByTestId('group-insights-tier-locked')).toBeInTheDocument();
    expect(screen.queryByTestId('group-report-summary')).toBeNull();
    expect(groupsApi.getWeeklyReport).not.toHaveBeenCalled();
    expect(groupsApi.getHealthFlags).not.toHaveBeenCalled();
  });

  it('offers Delete Group to the owner and archives it on confirm', async () => {
    const user = userEvent.setup();
    const { onMembershipEnded } = renderPanel();

    expect(screen.queryByTestId('group-info-leave')).toBeNull();
    await user.click(await screen.findByTestId('group-info-delete'));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Delete Group' }));

    await waitFor(() => expect(groupsApi.deleteGroup).toHaveBeenCalledWith(GROUP_ID));
    await waitFor(() => expect(onMembershipEnded).toHaveBeenCalledTimes(1));
  });

  it('offers Leave to a member and drops the thread once they are out', async () => {
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member', peer_sharing_consent: false })],
    });
    vi.mocked(groupsApi.leaveGroup).mockResolvedValue(undefined);
    const user = userEvent.setup();
    const { onMembershipEnded } = renderPanel();

    expect(await screen.findByTestId('group-info-leave')).toBeInTheDocument();
    expect(screen.queryByTestId('group-info-delete')).toBeNull();
    // A plain member sees no settings form either, the digest select included.
    expect(screen.queryByTestId('group-info-save-settings')).toBeNull();
    expect(screen.queryByTestId('group-digest-mode')).toBeNull();

    await user.click(screen.getByTestId('group-info-leave'));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Leave Group' }));

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

    renderPanel();

    const section = await screen.findByTestId('delegation-section');
    expect(await within(section).findByTestId('delegation-roster-row-900001')).toHaveTextContent(
      'Alex Athlete',
    );
    expect(screen.getByTestId('group-info-coach-badge')).toHaveTextContent('You coach this group');
    // The coach sees themself in the roster, marked as the viewer, under the agent.
    const coachRow = screen.getByTestId('group-info-coach-row');
    expect(within(coachRow).getByText('Caller Coach')).toBeInTheDocument();
    expect(within(coachRow).getByText('(you)')).toBeInTheDocument();
    expect(within(coachRow).getByText('Coach')).toBeInTheDocument();
    expect(within(screen.getByTestId('group-info-agent')).getByText('Marathon Coach')).toBeInTheDocument();
    expect(screen.queryByTestId('group-info-leave')).toBeNull();
    expect(screen.queryByTestId('group-info-delete')).toBeNull();
    expect(screen.queryByTestId('peer-consent-card')).toBeNull();
    // The coach sets where the weekly digest goes, and no other group setting.
    expect(await screen.findByTestId('group-digest-mode')).toHaveValue('off');
    expect(screen.getByTestId('group-info-save-settings')).toBeInTheDocument();
    expect(screen.queryByLabelText('Group Name')).toBeNull();
    expect(screen.queryByLabelText('Agent replies in the group chat')).toBeNull();
    expect(screen.queryByTestId('group-info-remove-coach')).toBeNull();
    // Stats and invites refuse a non-member, so the coach never asks for them.
    expect(groupsApi.getStats).not.toHaveBeenCalled();
    expect(groupsApi.listInvites).not.toHaveBeenCalled();
  });

  it("offers the group coach a way into each member's plan, and opens that thread", async () => {
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

    expect(await screen.findByRole('button', { name: "Work on Bob Cyclist's plan" })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: "Work on Alice Runner's plan" }));

    await waitFor(() => expect(onOpenConversation).toHaveBeenCalledWith('conv-plan-alice'));
    expect(chatApi.openAthleteThread).toHaveBeenCalledWith(OTHER_ID, GROUP_ID);
  });

  it("offers a coach who is also a member every other member's plan, never their own", async () => {
    // The caller owns the group, holds its membership row, and coaches it:
    // the server admits them for every other member and refuses them for
    // themselves.
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: CALLER_ID }));
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'coach' });
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'trainingpeaks', athletes: [] });
    renderPanel();

    expect(await screen.findByRole('button', { name: "Work on Other's plan" })).toBeInTheDocument();
    expect(screen.getByTestId(`member-work-on-plan-${OTHER_ID}`)).toBeInTheDocument();
    expect(screen.queryByTestId(`member-work-on-plan-${CALLER_ID}`)).toBeNull();
    expect(screen.queryByRole('button', { name: "Work on Caller's plan" })).toBeNull();
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

      await user.click(await screen.findByRole('button', { name: "Work on Alice Runner's plan" }));

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

      await user.click(await screen.findByRole('button', { name: "Work on Alice Runner's plan" }));

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
    expect(screen.queryByRole('button', { name: /Work on .* plan/ })).toBeNull();
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
    expect(screen.queryByRole('button', { name: /Work on .* plan/ })).toBeNull();
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

  it('lets a member who connects Intervals.icu themselves, with no link, allow calendar writes', async () => {
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup({ coach_user_id: COACH_ID, coach_display_name: 'Casey Coach' }));
    vi.mocked(groupsApi.listMembers).mockResolvedValue({
      members: [member({ role: 'member' })],
    });
    vi.mocked(groupsApi.listDelegatedConnections).mockResolvedValue({ connections: [], total: 0, viewer: 'member' });
    vi.mocked(groupsApi.getCoachConsents).mockResolvedValue(coachConsents({ calendar_write_supported: true }));
    vi.mocked(groupsApi.updateCoachConsents).mockResolvedValue(
      coachConsents({ calendar_write: true, calendar_write_supported: true }),
    );
    const user = userEvent.setup();
    renderPanel();

    const calendar = await screen.findByTestId('coach-consent-calendar_write');
    expect(screen.queryByTestId('delegation-linked')).toBeNull();
    expect(screen.getByTestId('coach-consent-calendar-step')).toHaveTextContent(
      'open intervals.icu/athletes, click Casey Coach and pick the Coach role, not Follower.',
    );
    expect(screen.queryByTestId('coach-consent-wellness_read')).toBeNull();
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
});
