// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Group info's roster and gating — the agent and human coach named above the members, admin rows, digest, exits
// ABOUTME: An owner, a plain member and the group's coach each get their own; a member answers the coach's TrainingPeaks link

import React from 'react';
import { render, fireEvent, waitFor, act, within } from '@testing-library/react-native';
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
  agent_title: 'Coach Marie',
  agent_handle: 'marie',
  owner_id: 'user-owner',
  coach_user_id: null,
  coach_display_name: null,
  peer_data_sharing: true,
  respond_mode: 'all',
  digest_mode: 'off',
  max_members: 12,
  is_active: true,
  created_at: '2026-05-01T08:00:00Z',
  updated_at: '2026-08-01T08:00:00Z',
};

/** The same group with Casey attached as its human coach. */
const COACHED_GROUP: CoachingGroup = { ...GROUP, coach_user_id: 'user-coach', coach_display_name: 'Casey Coach' };

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
import { coachesApi, groupsApi } from '../src/services/api';

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

  // Three kinds of people share a group, and the roster says which is which:
  // the AI agent answering in the chat, the human coach overseeing it, and
  // the members.
  describe('who runs the group', () => {
    it('names the agent and the coach above the members, for a plain member', async () => {
      mockCallerId = 'user-phil';
      (groupsApi.getGroup as jest.Mock).mockResolvedValue(COACHED_GROUP);
      const { findByTestId, getByTestId, queryByTestId } = renderSheet();

      const agent = await findByTestId('group-info-ai-coach');
      expect(agent).toHaveTextContent(/Coach Marie · @marie/);
      expect(agent).toHaveTextContent(/AI agent/);
      const coach = getByTestId('group-info-human-coach');
      expect(within(coach).getByText('Casey Coach')).toBeTruthy();
      // The role line under the name.
      expect(within(coach).getByText('Coach')).toBeTruthy();
      expect(coach).not.toHaveTextContent(/\(you\)/);
      // Detaching the coach is an admin's call; a member is not offered it.
      expect(queryByTestId('remove-coach-button')).toBeNull();
      expect(queryByTestId('group-info-no-coach')).toBeNull();

      // One list, in order: who runs the group, then who is in it.
      await findByTestId('group-member-user-phil');
      const order = within(getByTestId('group-info-members-content'))
        .getAllByTestId(/^group-(info-(ai|human)-coach|member-)/)
        .map((row) => row.props.testID);
      expect(order).toEqual([
        'group-info-ai-coach',
        'group-info-human-coach',
        'group-member-user-phil',
        'group-member-user-owner',
      ]);
    });

    it('tells a plain member the group has no human coach yet', async () => {
      mockCallerId = 'user-phil';
      const { findByTestId, queryByTestId } = renderSheet();

      const none = await findByTestId('group-info-no-coach');
      expect(within(none).getByText('No human coach in this group yet.')).toBeTruthy();
      expect(queryByTestId('group-info-human-coach')).toBeNull();
    });

    it('points an admin of a coach-less group at the coach invite', async () => {
      const { findByTestId } = renderSheet();

      const none = await findByTestId('group-info-no-coach');
      expect(within(none).getByText('No human coach attached. Share a coach invite to bring one in.')).toBeTruthy();
    });

    it('gives an admin the remove control on the coach row, and detaches the coach through it', async () => {
      (groupsApi.getGroup as jest.Mock).mockResolvedValue(COACHED_GROUP);
      (groupsApi.removeCoach as jest.Mock).mockResolvedValue(undefined);
      const { findByTestId } = renderSheet();

      const coach = await findByTestId('group-info-human-coach');
      expect(coach).toHaveTextContent(/Casey Coach/);
      fireEvent.press(within(coach).getByTestId('remove-coach-button'));

      const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
        string,
        string,
        Array<{ text: string; onPress?: () => Promise<void> }>,
      ];
      expect(confirm[0]).toBe('Remove Coach');
      await act(async () => {
        await confirm[2].find((button) => button.text === 'Remove')?.onPress?.();
      });
      expect(groupsApi.removeCoach).toHaveBeenCalledWith('group-1');
    });

    // A member who joined from another tenant has no such agent on their own
    // coach list; the group carries the title the server resolved in the
    // group's tenant, and the sheet reads that, not the caller's list.
    it("names the agent from the group, not from the caller's own coach list", async () => {
      mockCallerId = 'user-phil';
      (coachesApi.list as jest.Mock).mockResolvedValue({
        agents: [{ id: 'coach-elsewhere', title: 'Somebody Else', handle: 'else' }],
      });
      const { findByTestId } = renderSheet();

      const agent = await findByTestId('group-info-ai-coach');
      expect(agent).toHaveTextContent(/Coach Marie · @marie/);
      expect(agent).not.toHaveTextContent(/Somebody Else/);
      expect(coachesApi.list).not.toHaveBeenCalled();
    });

    it('calls an agent the server could not resolve "AI agent", with no handle', async () => {
      (groupsApi.getGroup as jest.Mock).mockResolvedValue({ ...GROUP, agent_title: null, agent_handle: null });
      const { findByTestId } = renderSheet();

      const agent = await findByTestId('group-info-ai-coach');
      expect(agent).toHaveTextContent(/AI agent/);
      expect(agent).not.toHaveTextContent(/@/);
    });
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
      (groupsApi.getGroup as jest.Mock).mockResolvedValue(COACHED_GROUP);
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
    (groupsApi.getGroup as jest.Mock).mockResolvedValue(COACHED_GROUP);
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
    // The coach holds no membership row; the roster still names them, as themself.
    expect(getByTestId('group-info-human-coach')).toHaveTextContent(/Casey Coach \(you\)/);
    expect(getByTestId('group-info-ai-coach')).toHaveTextContent(/Coach Marie · @marie/);
    expect(queryByTestId('group-member-user-coach')).toBeNull();
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
    (groupsApi.getGroup as jest.Mock).mockResolvedValue({
      ...GROUP,
      coach_user_id: 'user-owner',
      coach_display_name: 'ChefFamille',
    });
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
    expect(getByTestId('group-info-human-coach')).toHaveTextContent(/ChefFamille \(you\)/);
    // A member still: the owner keeps their row and their exit.
    expect(getByTestId('group-member-user-owner')).toHaveTextContent(/Owner/);
    expect(getByTestId('archive-group-button')).toBeTruthy();
  });

  it('asks a member to confirm the link their coach proposed, and confirms it', async () => {
    mockCallerId = 'user-phil';
    (groupsApi.getGroup as jest.Mock).mockResolvedValue(COACHED_GROUP);
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
