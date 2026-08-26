// ABOUTME: Unit tests for GroupDetailScreen's membership and admin-affordance derivation
// ABOUTME: Pins that members returned by the API are counted and that the owner keeps admin controls

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({ groupId: 'group-1' }),
}));

const OWNER_USER_ID = 'a1f4d8d9-7498-4881-82c3-4c5c13161561';

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    isAuthenticated: true,
    user: { id: OWNER_USER_ID, user_id: OWNER_USER_ID, email: 'alice@acme.com' },
  }),
}));

const mockMembers = jest.fn();
jest.mock('../src/hooks/useGroups', () => ({
  useGroup: () => ({
    group: {
      id: 'group-1',
      name: 'UX Validation Squad',
      description: null,
      coach_id: 'coach-1',
      owner_id: OWNER_USER_ID,
      is_active: true,
    },
    isLoading: false,
    isError: false,
    error: null,
    refetch: jest.fn(),
  }),
  useGroupMembers: () => ({
    members: mockMembers(),
    isLoading: false,
    isError: false,
    error: null,
    refetch: jest.fn(),
  }),
  useGroupStats: () => ({
    stats: null,
    isLoading: false,
    isError: false,
    error: null,
    refetch: jest.fn(),
  }),
  useGroupActions: () => ({
    leaveGroup: jest.fn(),
    isLeaving: false,
  }),
  // The admin controls this screen gained alongside the membership fix. Stubbed
  // inert so this file keeps testing exactly what it was written for: that the
  // caller's own row is found, and that every row the API returned is counted.
  useGroupInvites: () => ({ invites: [], isLoading: false, refetch: jest.fn() }),
  useUpdateGroup: () => ({ updateGroup: jest.fn(), isPending: false }),
  useDeleteGroup: () => ({ deleteGroup: jest.fn(), isPending: false }),
  useDeactivateInvite: () => ({ deactivateInvite: jest.fn(), isPending: false }),
  useUpdateMemberRole: () => ({ updateMemberRole: jest.fn(), isPending: false }),
  useUpdatePeerConsent: () => ({ updatePeerConsent: jest.fn(), isPending: false }),
  useGroupPermissions: () => ({ canCreate: true, policy: 'everyone', weeklyDigest: false }),
  useGroupWeeklyReport: () => ({ report: null, isLoading: false, refetch: jest.fn() }),
  useGroupTranscript: () => ({ transcript: { group_id: 'group-1', members: [], entries: [] }, isLoading: false, isError: false, refetch: jest.fn() }),
  useGroupHealthFlags: () => ({ flags: [], total: 0, isLoading: false, refetch: jest.fn() }),
}));

jest.mock('../src/services/api', () => ({
  groupsApi: {
    createInvite: jest.fn(),
    removeMember: jest.fn(),
    removeCoach: jest.fn(),
  },
  chatApi: {
    createConversation: jest.fn(),
  },
}));

import { GroupDetailScreen } from '../src/screens/groups/GroupDetailScreen';

/**
 * A member exactly as the members endpoint serialises it. The server lists
 * active members only, so no `left_at` is sent — a client filter on that field
 * dropped every row and left the owner with no admin controls on their own group.
 */
function apiMember(overrides: Record<string, unknown> = {}) {
  return {
    id: 'membership-1',
    group_id: 'group-1',
    user_id: OWNER_USER_ID,
    role: 'owner',
    peer_sharing_consent: false,
    consent_given_at: '2026-08-24T19:46:51.548903+00:00',
    joined_at: '2026-08-24T19:46:51.548903+00:00',
    display_name: 'alice@acme.com',
    ...overrides,
  };
}

describe('GroupDetailScreen membership derivation', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockMembers.mockReturnValue([apiMember()]);
  });

  it('counts the members the API returned', async () => {
    const { getByText } = render(<GroupDetailScreen />);

    await waitFor(() => {
      expect(getByText('Members (1)')).toBeTruthy();
    });
  });

  it('keeps the invite control for the owner of the group', async () => {
    const { getByTestId } = render(<GroupDetailScreen />);

    await waitFor(() => {
      expect(getByTestId('share-invite-button')).toBeTruthy();
    });
  });

  it('lists a second member alongside the owner', async () => {
    mockMembers.mockReturnValue([
      apiMember(),
      apiMember({
        id: 'membership-2',
        user_id: 'user-2',
        role: 'member',
        display_name: 'phil@dravr.ai',
      }),
    ]);

    const { getByText } = render(<GroupDetailScreen />);

    await waitFor(() => {
      expect(getByText('Members (2)')).toBeTruthy();
      expect(getByText('phil@dravr.ai')).toBeTruthy();
    });
  });

  it('withholds the invite control from a plain member', async () => {
    mockMembers.mockReturnValue([
      apiMember({ id: 'membership-2', user_id: 'someone-else', role: 'owner' }),
      apiMember({ id: 'membership-3', role: 'member' }),
    ]);

    const { queryByTestId } = render(<GroupDetailScreen />);

    await waitFor(() => {
      expect(queryByTestId('share-invite-button')).toBeNull();
    });
  });
});
