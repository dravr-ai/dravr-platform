// ABOUTME: Tests for Group info inside chat — consent, roster, settings, the digest mode and gate, the exits and the coach's view
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
import type { CoachingGroup, DelegatedConnection, GroupMember } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';

const CALLER_ID = 'user-caller';
const OTHER_ID = 'user-other';
const GROUP_ID = 'group-1';
const COACH_ID = 'user-coach';

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
    endDelegatedConnection: vi.fn(),
  },
  providersApi: {
    getProvidersStatus: vi.fn(),
  },
}));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: CALLER_ID, email: 'caller@example.com' } }),
}));

const { groupsApi } = await import('../../../services/api');

function sampleGroup(overrides: Partial<CoachingGroup> = {}): CoachingGroup {
  return {
    id: GROUP_ID,
    tenant_id: 'tenant-a',
    name: 'Marathon Squad',
    description: 'Sunday long runs',
    agent_id: 'coach-1',
    coach_user_id: null,
    owner_id: CALLER_ID,
    max_members: 10,
    peer_data_sharing: true,
    respond_mode: 'all',
    digest_mode: 'off',
    is_active: true,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  } as CoachingGroup;
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

function renderPanel() {
  const onMembershipEnded = vi.fn();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const utils = render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <GroupInfoPanel groupId={GROUP_ID} onMembershipEnded={onMembershipEnded} />
      </ToastProvider>
    </QueryClientProvider>,
  );
  return { ...utils, onMembershipEnded };
}

describe('GroupInfoPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(groupsApi.getGroup).mockResolvedValue(sampleGroup());
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
      sampleGroup({ coach_user_id: CALLER_ID, owner_id: OTHER_ID }),
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
      expect(groupsApi.confirmDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, 'dc-1'),
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
});
