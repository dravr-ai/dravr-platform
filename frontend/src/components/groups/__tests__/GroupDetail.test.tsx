// ABOUTME: Tests for GroupDetail's consent switch, group-scoped chat, and insights panel
// ABOUTME: Pins that the caller's OWN row is bound and that group_id reaches createConversation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import GroupDetail from '../GroupDetail';
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
  },
  chatApi: {
    createConversation: vi.fn(),
  },
}));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: CALLER_ID, email: 'caller@example.com' } }),
}));

const { groupsApi, chatApi } = await import('../../../services/api');

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

function renderDetail() {
  const onNavigate = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const utils = render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <GroupDetail groupId={GROUP_ID} onBack={vi.fn()} onNavigate={onNavigate} />
      </ToastProvider>
    </QueryClientProvider>,
  );
  return { ...utils, onNavigate };
}

describe('GroupDetail', () => {
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
    vi.mocked(groupsApi.listInvites).mockResolvedValue({ invites: [] });
  });

  it('binds the consent switch to the caller own membership row', async () => {
    renderDetail();

    const toggle = await screen.findByTestId('peer-consent-switch');
    // The caller's own row has consent off; the other member's row has it on.
    expect((toggle as HTMLInputElement).checked).toBe(false);
  });

  it('sends the new consent value when the caller flips the switch', async () => {
    renderDetail();

    const toggle = await screen.findByTestId('peer-consent-switch');
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(groupsApi.updatePeerConsent).toHaveBeenCalledWith(GROUP_ID, { consent: true });
    });
    expect(groupsApi.updatePeerConsent).toHaveBeenCalledTimes(1);
  });

  it('opens a group-scoped conversation and routes to it', async () => {
    vi.mocked(chatApi.createConversation).mockResolvedValue({
      id: 'conv-9',
      title: 'Marathon Squad',
      group_id: GROUP_ID,
      coach_id: 'coach-1',
      message_count: 0,
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-01-01T00:00:00Z',
    });

    const { onNavigate } = renderDetail();

    fireEvent.click(await screen.findByTestId('group-chat-button'));

    await waitFor(() => {
      expect(chatApi.createConversation).toHaveBeenCalledWith({
        title: 'Marathon Squad',
        coach_id: 'coach-1',
        group_id: GROUP_ID,
      });
    });
    await waitFor(() => {
      expect(onNavigate).toHaveBeenCalledWith('chat/conv-9');
    });
  });

  it('renders the weekly report and one health-flag row per flagged member', async () => {
    renderDetail();

    fireEvent.click(await screen.findByRole('tab', { name: /Stats/i }));

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

    renderDetail();

    fireEvent.click(await screen.findByRole('tab', { name: /Stats/i }));

    expect(await screen.findByTestId('group-insights-tier-locked')).toBeInTheDocument();
    expect(screen.queryByTestId('group-report-summary')).toBeNull();
    expect(groupsApi.getWeeklyReport).not.toHaveBeenCalled();
    expect(groupsApi.getHealthFlags).not.toHaveBeenCalled();
  });
});
