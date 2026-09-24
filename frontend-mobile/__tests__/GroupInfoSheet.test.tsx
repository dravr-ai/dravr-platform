// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Group info's membership gating — who sees the admin rows and the digest mode, who leaves, who archives
// ABOUTME: An owner, a plain member and the group's coach each get their own; a member answers the coach's TrainingPeaks link

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachingGroup, GroupMember, GroupRole } from '@pierre/shared-types';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn() }),
}));

const GROUP: CoachingGroup = {
  id: 'group-1',
  tenant_id: 'tenant-1',
  name: 'Harricana 2027',
  description: 'Bloc ultra',
  agent_id: 'coach-1',
  owner_id: 'user-owner',
  coach_user_id: null,
  peer_data_sharing: true,
  respond_mode: 'all',
  digest_mode: 'off',
  max_members: 12,
  is_active: true,
  created_at: '2026-05-01T08:00:00Z',
  updated_at: '2026-08-01T08:00:00Z',
};

const mockLeaveGroup = jest.fn();
const mockDeleteGroup = jest.fn();

const mockConfirmLink = jest.fn();
jest.mock('../src/services/api', () => ({
  coachesApi: { list: jest.fn().mockResolvedValue({ agents: [] }) },
  oauthApi: { getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }) },
  groupsApi: {
    getGroup: jest.fn(),
    listDelegatedConnections: jest.fn(),
    getDelegationRoster: jest.fn(),
    proposeDelegatedConnection: jest.fn(),
    confirmDelegatedConnection: (...args: unknown[]) => mockConfirmLink(...args),
    endDelegatedConnection: jest.fn(),
    listMembers: jest.fn(),
    getStats: jest.fn().mockResolvedValue({ stats: null }),
    listInvites: jest.fn().mockResolvedValue({ invites: [] }),
    getPermissions: jest.fn(),
    getTranscript: jest.fn().mockResolvedValue({ group_id: 'group-1', entries: [] }),
    getWeeklyReport: jest.fn(),
    getHealthFlags: jest.fn(),
    leaveGroup: (...args: unknown[]) => mockLeaveGroup(...args),
    deleteGroup: (...args: unknown[]) => mockDeleteGroup(...args),
    removeMember: jest.fn(),
    removeCoach: jest.fn(),
    updateGroup: jest.fn(),
    updateMemberRole: jest.fn(),
    updatePeerConsent: jest.fn(),
    createInvite: jest.fn(),
    deactivateInvite: jest.fn(),
  },
}));

let mockCallerId = 'user-owner';
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: mockCallerId }, isAuthenticated: true }),
}));

import { GroupInfoSheet } from '../src/screens/groups/GroupInfoSheet';
import { groupsApi } from '../src/services/api';

function member(id: string, role: GroupRole, name: string): GroupMember {
  return {
    id: `member-${id}`,
    group_id: 'group-1',
    user_id: id,
    role,
    peer_sharing_consent: true,
    consent_given_at: '2026-05-01T08:00:00Z',
    joined_at: '2026-05-01T08:00:00Z',
    display_name: name,
  };
}

const MEMBERS = [member('user-phil', 'member', 'Phil'), member('user-owner', 'owner', 'ChefFamille')];

function renderSheet() {
  const handlers = { onClose: jest.fn(), onLeft: jest.fn() };
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <GroupInfoSheet groupId="group-1" fallbackName="Harricana" {...handlers} />
    </QueryClientProvider>,
  );
  return { ...view, handlers };
}

describe('GroupInfoSheet', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockCallerId = 'user-owner';
    (groupsApi.getGroup as jest.Mock).mockResolvedValue(GROUP);
    (groupsApi.getPermissions as jest.Mock).mockResolvedValue({
      can_create: true,
      policy: 'everyone',
      weekly_digest: false,
    });
    (groupsApi.listMembers as jest.Mock).mockResolvedValue({ members: MEMBERS });
    (groupsApi.listDelegatedConnections as jest.Mock).mockResolvedValue({
      connections: [],
      total: 0,
      viewer: 'member',
    });
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => jest.restoreAllMocks());

  it('names the group, counts its members and draws one row each', async () => {
    const { findByTestId, getByTestId } = renderSheet();

    expect(await findByTestId('group-info-name')).toHaveTextContent('Harricana 2027');
    expect(getByTestId('group-info-description')).toHaveTextContent('Bloc ultra');
    // The row carries the avatar initials and the role badge beside the name,
    // so the assertion is on the name appearing in that row, not the row text.
    expect(getByTestId('group-member-user-phil')).toHaveTextContent(/Phil/);
    expect(getByTestId('group-member-user-phil')).toHaveTextContent(/Member/);
    expect(getByTestId('group-member-user-owner')).toHaveTextContent(/ChefFamille/);
    expect(getByTestId('group-member-user-owner')).toHaveTextContent(/Owner/);
  });

  // An owner cannot leave their own group; they archive it. A member cannot
  // archive it; they leave. Offering the wrong one advertises a refusal.
  it('offers the owner Archive and no Leave', async () => {
    const { findByTestId, queryByTestId } = renderSheet();

    expect(await findByTestId('archive-group-button')).toBeTruthy();
    await waitFor(() => expect(queryByTestId('leave-group-button')).toBeNull());
  });

  it('archives the group and sends the athlete back to the list', async () => {
    mockDeleteGroup.mockResolvedValue(undefined);
    const { findByTestId, handlers } = renderSheet();

    fireEvent.press(await findByTestId('archive-group-button'));
    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe('Archive Group');
    await act(async () => {
      await confirm[2].find((button) => button.text === 'Archive')?.onPress?.();
    });

    expect(mockDeleteGroup).toHaveBeenCalledWith('group-1');
    expect(handlers.onLeft).toHaveBeenCalledTimes(1);
  });

  it('offers a plain member Leave, the invite section and no admin settings', async () => {
    mockCallerId = 'user-phil';
    const { findByTestId, queryByTestId } = renderSheet();

    expect(await findByTestId('leave-group-button')).toBeTruthy();
    await waitFor(() => expect(queryByTestId('archive-group-button')).toBeNull());
    expect(queryByTestId('group-info-invites')).toBeNull();
  });

  describe('the weekly digest mode', () => {
    beforeEach(() => {
      (groupsApi.getPermissions as jest.Mock).mockResolvedValue({
        can_create: true,
        policy: 'everyone',
        weekly_digest: true,
      });
      (groupsApi.updateGroup as jest.Mock).mockResolvedValue({ ...GROUP, digest_mode: 'managers' });
    });

    async function openSettings(view: ReturnType<typeof renderSheet>) {
      await act(async () => {
        fireEvent.press(await view.findByTestId('group-info-settings-toggle'));
      });
    }

    it('offers the owner the three modes, checks the one in force and writes the one tapped', async () => {
      const view = renderSheet();
      await openSettings(view);

      const off = await view.findByTestId('group-digest-mode-off');
      expect(off.props.accessibilityState).toEqual({ selected: true });
      expect(view.getByTestId('group-digest-mode-chat').props.accessibilityState).toEqual({ selected: false });
      expect(view.getByTestId('group-digest-mode')).toHaveTextContent(/Weekly recap/);
      expect(view.getByTestId('group-digest-mode-managers')).toHaveTextContent(
        /Owner and admins only.*nothing is posted in the group's chat/,
      );

      await act(async () => {
        fireEvent.press(view.getByTestId('group-digest-mode-managers'));
      });
      await waitFor(() =>
        expect(groupsApi.updateGroup).toHaveBeenCalledWith('group-1', { digest_mode: 'managers' }),
      );
    });

    it('gives the attached coach the digest rows without the admin settings', async () => {
      mockCallerId = 'user-coach';
      (groupsApi.getGroup as jest.Mock).mockResolvedValue({ ...GROUP, coach_user_id: 'user-coach' });
      const view = renderSheet();
      await openSettings(view);

      expect(await view.findByTestId('group-digest-mode-chat')).toBeTruthy();
      expect(view.queryByTestId('group-name-input')).toBeNull();
      expect(view.queryByTestId('group-respond-mode-switch')).toBeNull();

      await act(async () => {
        fireEvent.press(view.getByTestId('group-digest-mode-chat'));
      });
      await waitFor(() =>
        expect(groupsApi.updateGroup).toHaveBeenCalledWith('group-1', { digest_mode: 'chat' }),
      );
    });

    it('shows no digest rows to a plain member', async () => {
      mockCallerId = 'user-phil';
      const view = renderSheet();
      await openSettings(view);

      await view.findByTestId('peer-consent-row');
      expect(view.queryByTestId('group-digest-mode')).toBeNull();
    });

    it('shows no digest rows when the tier sends no digest', async () => {
      (groupsApi.getPermissions as jest.Mock).mockResolvedValue({
        can_create: true,
        policy: 'everyone',
        weekly_digest: false,
      });
      const view = renderSheet();
      await openSettings(view);

      await view.findByTestId('group-respond-mode-row');
      expect(view.queryByTestId('group-digest-mode')).toBeNull();
    });
  });

  it('leaves the group and sends the athlete back to the list', async () => {
    mockCallerId = 'user-phil';
    mockLeaveGroup.mockResolvedValue(undefined);
    const { findByTestId, handlers } = renderSheet();

    fireEvent.press(await findByTestId('leave-group-button'));
    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe('Leave Group');
    await act(async () => {
      await confirm[2].find((button) => button.text === 'Leave')?.onPress?.();
    });

    expect(mockLeaveGroup).toHaveBeenCalledWith('group-1');
    expect(handlers.onLeft).toHaveBeenCalledTimes(1);
  });

  it("shows the group's coach the TrainingPeaks section and no exit, settings or analytics", async () => {
    mockCallerId = 'user-coach';
    (groupsApi.getGroup as jest.Mock).mockResolvedValue({ ...GROUP, coach_user_id: 'user-coach' });
    (groupsApi.listDelegatedConnections as jest.Mock).mockResolvedValue({
      connections: [],
      total: 0,
      viewer: 'coach',
    });
    (groupsApi.getDelegationRoster as jest.Mock).mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        { provider_athlete_id: '900001', display_name: 'Alex Athlete', connection: null, suggested_member_user_id: 'user-phil' },
      ],
    });
    const { findByTestId, queryByTestId, getByTestId } = renderSheet();

    expect(await findByTestId('group-info-delegation')).toBeTruthy();
    expect(await findByTestId('delegation-roster-row-900001')).toHaveTextContent(/Alex Athlete/);
    fireEvent.press(getByTestId('group-info-coach-toggle'));
    expect(getByTestId('group-info-human-coach')).toHaveTextContent(/You coach this group/);
    expect(queryByTestId('leave-group-button')).toBeNull();
    expect(queryByTestId('archive-group-button')).toBeNull();
    expect(queryByTestId('group-info-settings')).toBeNull();
    expect(queryByTestId('group-info-analytics')).toBeNull();
    // Stats and invites refuse a non-member, so the coach never asks for them.
    expect(groupsApi.getStats).not.toHaveBeenCalled();
    expect(groupsApi.listInvites).not.toHaveBeenCalled();
  });

  it('shows a coach who is also a member the TrainingPeaks section the server names them coach of', async () => {
    // The caller owns the group, holds its membership row, and coaches it.
    (groupsApi.getGroup as jest.Mock).mockResolvedValue({ ...GROUP, coach_user_id: 'user-owner' });
    (groupsApi.listDelegatedConnections as jest.Mock).mockResolvedValue({
      connections: [],
      total: 0,
      viewer: 'coach',
    });
    (groupsApi.getDelegationRoster as jest.Mock).mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        { provider_athlete_id: '900001', display_name: 'Alex Athlete', connection: null, suggested_member_user_id: null },
      ],
    });
    const { findByTestId, getByTestId } = renderSheet();

    expect(await findByTestId('group-info-delegation')).toBeTruthy();
    expect(await findByTestId('delegation-roster-row-900001')).toHaveTextContent(/Alex Athlete/);
    fireEvent.press(getByTestId('group-info-coach-toggle'));
    expect(getByTestId('group-info-human-coach')).toHaveTextContent(/You coach this group/);
    // A member still: the owner keeps their exit.
    expect(getByTestId('archive-group-button')).toBeTruthy();
  });

  it('asks a member to confirm the link their coach proposed, and confirms it', async () => {
    mockCallerId = 'user-phil';
    (groupsApi.getGroup as jest.Mock).mockResolvedValue({ ...GROUP, coach_user_id: 'user-coach' });
    (groupsApi.listDelegatedConnections as jest.Mock).mockResolvedValue({
      connections: [
        {
          id: 'dc-1',
          group_id: 'group-1',
          provider: 'trainingpeaks',
          coach_user_id: 'user-coach',
          coach_display_name: 'Casey Coach',
          member_user_id: 'user-phil',
          member_display_name: 'Phil',
          provider_athlete_id: '900001',
          provider_athlete_name: 'Alex Athlete',
          status: 'proposed',
          proposed_at: '2026-09-24T08:00:00Z',
          confirmed_at: null,
        },
      ],
      total: 1,
      viewer: 'member',
    });
    mockConfirmLink.mockResolvedValue({});
    const { findByTestId, getByTestId } = renderSheet();

    expect(await findByTestId('delegation-request-body')).toHaveTextContent(
      "Casey Coach coaches you on TrainingPeaks as Alex Athlete. Confirm, and Dravr reads your TrainingPeaks workouts through Casey Coach's account — you do not sign in to TrainingPeaks yourself.",
    );
    await act(async () => {
      fireEvent.press(getByTestId('delegation-confirm'));
    });
    expect(mockConfirmLink).toHaveBeenCalledWith('group-1', 'dc-1');
  });
});
