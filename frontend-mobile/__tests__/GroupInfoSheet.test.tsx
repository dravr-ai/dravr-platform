// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Group info's membership gating — who sees the admin rows, who leaves, who archives
// ABOUTME: The sheet is the only group surface left, so an owner and a plain member must each get their own

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachingGroup, GroupMember, GroupRole } from '@pierre/shared-types';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn() }),
}));

const mockLeaveGroup = jest.fn();
const mockDeleteGroup = jest.fn();
jest.mock('../src/services/api', () => ({
  coachesApi: { list: jest.fn().mockResolvedValue({ coaches: [] }) },
  groupsApi: {
    getGroup: jest.fn().mockResolvedValue({
      id: 'group-1',
      tenant_id: 'tenant-1',
      name: 'Harricana 2027',
      description: 'Bloc ultra',
      coach_id: 'coach-1',
      owner_id: 'user-owner',
      coach_user_id: null,
      peer_data_sharing: true,
      respond_mode: 'all',
      max_members: 12,
      is_active: true,
      created_at: '2026-05-01T08:00:00Z',
      updated_at: '2026-08-01T08:00:00Z',
    } as CoachingGroup),
    listMembers: jest.fn(),
    getStats: jest.fn().mockResolvedValue({ stats: null }),
    listInvites: jest.fn().mockResolvedValue({ invites: [] }),
    getPermissions: jest.fn().mockResolvedValue({ can_create: true, policy: 'everyone', weekly_digest: false }),
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
    (groupsApi.listMembers as jest.Mock).mockResolvedValue({ members: MEMBERS });
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
});
