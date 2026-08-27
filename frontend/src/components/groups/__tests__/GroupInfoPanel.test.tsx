// ABOUTME: Tests for Group info inside chat — consent, roster, settings, the digest gate and the exits
// ABOUTME: Carries over the GroupDetail cases: the caller's OWN consent row, and the tier-gated report
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import GroupInfoPanel from '../GroupInfoPanel';
import { ToastProvider } from '../../ui';
import type { CoachingGroup, GroupMember } from '@pierre/shared-types';

const CALLER_ID = 'user-caller';
const OTHER_ID = 'user-other';
const GROUP_ID = 'group-1';

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
    coach_id: 'coach-1',
    coach_user_id: null,
    owner_id: CALLER_ID,
    max_members: 10,
    peer_data_sharing: true,
    respond_mode: 'all',
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
        summary:
          'Marathon Squad had 2/2 active members this week with average volume of 41.5km. Overall trend: stable.',
        highlights: ['Caller is in fresh form (TSB +9, 12% of CTL)'],
        concerns: ['Other: no activity for 11 days'],
        recommendations: ['Review 1 flagged member(s) and consider recovery adjustments.'],
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
          detail: 'no activity for 11 days',
        },
        {
          user_id: CALLER_ID,
          display_name: 'Caller',
          flag_type: 'volume_drop',
          severity: 'warning',
          detail: 'weekly volume down 35%',
        },
      ],
      total: 2,
    });
    vi.mocked(groupsApi.updatePeerConsent).mockResolvedValue(undefined);
    vi.mocked(groupsApi.updateGroup).mockResolvedValue(sampleGroup());
    vi.mocked(groupsApi.deleteGroup).mockResolvedValue(undefined);
    vi.mocked(groupsApi.listInvites).mockResolvedValue({ invites: [] });
    vi.mocked(groupsApi.getTranscript).mockResolvedValue({ entries: [], total: 0 });
  });

  it('names the group, its description and its roster size', async () => {
    renderPanel();

    expect(await screen.findByTestId('group-info-name')).toHaveTextContent('Marathon Squad');
    expect(screen.getByTestId('group-info-description')).toHaveTextContent('Sunday long runs');
    expect(await screen.findByText('2 members')).toBeInTheDocument();
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
      }),
    );
  });

  it('renders the weekly report and one health-flag row per flagged member', async () => {
    renderPanel();

    expect(await screen.findByTestId('group-report-summary')).toHaveTextContent(
      'Marathon Squad had 2/2 active members this week',
    );
    expect(screen.getAllByTestId('group-report-highlight')).toHaveLength(1);
    expect(screen.getAllByTestId('group-report-concern')).toHaveLength(1);
    expect(screen.getAllByTestId('group-report-recommendation')).toHaveLength(1);
    expect(screen.getAllByTestId('group-health-flag-row')).toHaveLength(2);
    expect(screen.getByText('Health flags (2)')).toBeInTheDocument();
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
    // A plain member sees no settings form either.
    expect(screen.queryByTestId('group-info-save-settings')).toBeNull();

    await user.click(screen.getByTestId('group-info-leave'));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Leave Group' }));

    await waitFor(() => expect(groupsApi.leaveGroup).toHaveBeenCalledWith(GROUP_ID));
    await waitFor(() => expect(onMembershipEnded).toHaveBeenCalledTimes(1));
  });
});
