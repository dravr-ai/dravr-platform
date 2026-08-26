// ABOUTME: carnet #55/#52/#53/#60 e2e — mobile group admin controls, consent, group chat, weekly report
// ABOUTME: Asserts the consent PUT carries the real value and the group chat POST carries group_id

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type {
  CoachingGroup,
  GroupHealthFlagsResponse,
  GroupInvitesResponse,
  GroupMembersResponse,
  GroupPermissionsResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
} from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

const mockBack = jest.fn();
const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({
    push: mockPush,
    replace: jest.fn(),
    back: mockBack,
    navigate: jest.fn(),
    canGoBack: () => true,
  }),
  useLocalSearchParams: () => ({ groupId: 'group-1' }),
  useSegments: () => [],
  usePathname: () => '/groups/group-1',
  useFocusEffect: () => undefined,
}));

// The caller is the group owner, and — deliberately — NOT the first member the
// server lists. Binding the consent switch to the first active row instead
// would show and write Phil's consent from ChefFamille's phone.
jest.mock('../../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    user: { id: 'user-owner', email: 'chef@dravr.ai', role: 'user', user_status: 'active' },
    isAuthenticated: true,
  }),
}));

import { GroupDetailScreen } from '../../src/screens/groups/GroupDetailScreen';

const GROUP: CoachingGroup = {
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
};

function membersResponse(ownerConsent: boolean): GroupMembersResponse {
  return {
    members: [
      {
        id: 'member-phil',
        group_id: 'group-1',
        user_id: 'user-phil',
        role: 'member',
        peer_sharing_consent: true,
        consent_given_at: '2026-05-02T08:00:00Z',
        joined_at: '2026-05-02T08:00:00Z',
        display_name: 'Phil',
      },
      {
        id: 'member-owner',
        group_id: 'group-1',
        user_id: 'user-owner',
        role: 'owner',
        peer_sharing_consent: ownerConsent,
        consent_given_at: '2026-05-01T08:00:00Z',
        joined_at: '2026-05-01T08:00:00Z',
        display_name: 'ChefFamille',
      },
    ],
  };
}

const STATS: GroupStatsResponse = {
  stats: {
    total_members: 2,
    active_members: 2,
    avg_weekly_volume_km: 61.5,
    avg_ctl: 48.2,
    flagged_members: 0,
    weekly_trend: 'stable',
  },
};

/** The tenant tier gate the digest scheduler reads, as the route serialises it. */
const PERMISSIONS: GroupPermissionsResponse = {
  can_create: true,
  policy: 'everyone',
  weekly_digest: true,
};

const REPORT: GroupWeeklyReportResponse = {
  report: {
    summary: 'Harricana 2027 had 2/2 active members this week with average volume of 61.5km.',
    highlights: ['Phil is in fresh form (TSB +9, 12% of CTL)'],
    concerns: ['ChefFamille: weekly volume down 35%'],
    recommendations: ['Review 1 flagged member(s) and consider recovery adjustments.'],
    stats: {
      total_members: 2,
      active_members: 2,
      avg_weekly_volume_km: 61.5,
      avg_ctl: 48.2,
      flagged_members: 1,
      weekly_trend: 'stable',
    },
  },
};

const HEALTH: GroupHealthFlagsResponse = {
  flags: [
    {
      user_id: 'user-owner',
      display_name: 'ChefFamille',
      flag_type: 'volume_drop',
      severity: 'warning',
      detail: 'weekly volume down 35%',
    },
    {
      user_id: 'user-phil',
      display_name: 'Phil',
      flag_type: 'inactive',
      severity: 'warning',
      detail: 'no activity for 11 days',
    },
  ],
  total: 2,
};

const INVITES: GroupInvitesResponse = {
  invites: [
    {
      id: 'invite-1',
      group_id: 'group-1',
      tenant_id: 'tenant-1',
      code: 'HARRI-7X2',
      kind: 'member',
      created_by: 'user-owner',
      expires_at: '2026-09-01T08:00:00Z',
      max_uses: null,
      use_count: 3,
      is_active: true,
      created_at: '2026-08-01T08:00:00Z',
    },
  ],
};

function renderGroup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <GroupDetailScreen />
    </QueryClientProvider>,
  );
}

describe('carnet #55/#52 — mobile group admin + peer consent', () => {
  let stub: HttpStub;
  let ownerConsent: boolean;

  beforeEach(() => {
    ownerConsent = false;
    mockBack.mockClear();
    mockPush.mockClear();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    stub = installHttpStub({
      'GET /api/groups/group-1': { data: GROUP },
      'GET /api/groups/group-1/members': () => ({ data: membersResponse(ownerConsent) }),
      'GET /api/groups/group-1/stats': { data: STATS },
      'GET /api/groups/group-1/invites': { data: INVITES },
      'GET /api/groups/permissions': { data: PERMISSIONS },
      'GET /api/groups/group-1/report': { data: REPORT },
      'GET /api/groups/group-1/health': { data: HEALTH },
      'POST /api/chat/conversations': (request) => ({
        status: 201,
        data: {
          id: 'conv-group-1',
          ...(request.body as Record<string, unknown>),
          total_tokens: 0,
          message_count: 0,
          created_at: '2026-08-25T08:00:00Z',
          updated_at: '2026-08-25T08:00:00Z',
        },
      }),
      'PUT /api/groups/group-1/members/me/consent': (request) => {
        ownerConsent = (request.body as { consent: boolean }).consent;
        return { data: { success: true } };
      },
      'PUT /api/groups/group-1/members/user-phil/role': { data: { success: true } },
      'DELETE /api/groups/group-1/invites/invite-1': { data: { success: true } },
    });
  });

  afterEach(() => {
    stub.restore();
    jest.restoreAllMocks();
  });

  it('binds the consent switch to the caller row and writes the real value', async () => {
    const { getByTestId } = renderGroup();

    await waitFor(() => {
      expect(getByTestId('peer-consent-switch')).toBeTruthy();
    });

    // ChefFamille has not consented; Phil (listed first) has. Reading the first
    // row would have shown `true` here.
    expect(getByTestId('peer-consent-switch').props.value).toBe(false);

    await act(async () => {
      fireEvent(getByTestId('peer-consent-switch'), 'valueChange', true);
    });

    const consentRequest = stub
      .requestsFor('PUT')
      .find((request) => request.url === '/api/groups/group-1/members/me/consent');
    expect(consentRequest?.body).toEqual({ consent: true });

    // Re-read shows the stored value, so the switch reflects the server.
    await waitFor(() => {
      expect(getByTestId('peer-consent-switch').props.value).toBe(true);
    });
  });

  it('promotes a member through the owner-only role control', async () => {
    const { getByTestId } = renderGroup();

    await waitFor(() => {
      expect(getByTestId('member-role-user-phil')).toBeTruthy();
    });

    await act(async () => {
      fireEvent.press(getByTestId('member-role-user-phil'));
    });

    await waitFor(() => {
      const roleRequest = stub
        .requestsFor('PUT')
        .find((request) => request.url === '/api/groups/group-1/members/user-phil/role');
      expect(roleRequest?.body).toEqual({ role: 'admin' });
    });
  });

  it('lists live invites in the admin sheet and deactivates one', async () => {
    const { getByTestId, getByText } = renderGroup();

    await waitFor(() => {
      expect(getByTestId('group-admin-button')).toBeTruthy();
    });

    await act(async () => {
      fireEvent.press(getByTestId('group-admin-button'));
    });

    await waitFor(() => {
      expect(getByTestId('group-invite-invite-1')).toBeTruthy();
    });
    expect(getByText('HARRI-7X2')).toBeTruthy();
    expect(getByText('Member invite · used 3×')).toBeTruthy();
    expect(getByText('Active Invites (1)')).toBeTruthy();

    fireEvent.press(getByTestId('deactivate-invite-invite-1'));

    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe('Deactivate Invite');
    await act(async () => {
      await confirm[2].find((button) => button.text === 'Deactivate')?.onPress?.();
    });

    await waitFor(() => {
      expect(stub.requestsFor('DELETE').map((request) => request.url)).toEqual([
        '/api/groups/group-1/invites/invite-1',
      ]);
    });
  });

  it('writes the group respond mode from the admin sheet', async () => {
    stub.restore();
    stub = installHttpStub({
      'GET /api/groups/group-1': { data: GROUP },
      'GET /api/groups/group-1/members': () => ({ data: membersResponse(ownerConsent) }),
      'GET /api/groups/group-1/stats': { data: STATS },
      'GET /api/groups/group-1/invites': { data: INVITES },
      'GET /api/groups/permissions': { data: PERMISSIONS },
      'GET /api/groups/group-1/report': { data: REPORT },
      'GET /api/groups/group-1/health': { data: HEALTH },
      'POST /api/chat/conversations': (request) => ({
        status: 201,
        data: {
          id: 'conv-group-1',
          ...(request.body as Record<string, unknown>),
          total_tokens: 0,
          message_count: 0,
          created_at: '2026-08-25T08:00:00Z',
          updated_at: '2026-08-25T08:00:00Z',
        },
      }),
      'PUT /api/groups/group-1': { data: { ...GROUP, respond_mode: 'mentions' } },
    });

    const { getByTestId } = renderGroup();

    await waitFor(() => {
      expect(getByTestId('group-admin-button')).toBeTruthy();
    });
    await act(async () => {
      fireEvent.press(getByTestId('group-admin-button'));
    });
    await waitFor(() => {
      expect(getByTestId('group-respond-mode-switch')).toBeTruthy();
    });
    expect(getByTestId('group-respond-mode-switch').props.value).toBe(false);

    await act(async () => {
      fireEvent(getByTestId('group-respond-mode-switch'), 'valueChange', true);
    });

    await waitFor(() => {
      const updateRequest = stub
        .requestsFor('PUT')
        .find((request) => request.url === '/api/groups/group-1');
      expect(updateRequest?.body).toEqual({ respond_mode: 'mentions' });
    });
  });

  it('opens a group-scoped conversation carrying group_id and the group name', async () => {
    const { getByTestId } = renderGroup();

    await waitFor(() => {
      expect(getByTestId('chat-with-coach-button')).toBeTruthy();
    });

    await act(async () => {
      fireEvent.press(getByTestId('chat-with-coach-button'));
    });

    const createRequest = stub
      .requestsFor('POST')
      .find((request) => request.url === '/api/chat/conversations');
    expect(createRequest?.body).toEqual({
      title: 'Harricana 2027',
      coach_id: 'coach-1',
      group_id: 'group-1',
    });

    await waitFor(() => {
      expect(mockPush).toHaveBeenCalledWith({
        pathname: '/(app)/(tabs)/(chat)',
        params: { conversationId: 'conv-group-1' },
      });
    });
  });

  it('renders the weekly report and one row per flagged member', async () => {
    const { getByTestId, getAllByTestId, getByText } = renderGroup();

    await waitFor(() => {
      expect(getByTestId('group-report-summary').props.children).toBe(REPORT.report.summary);
    });
    expect(getAllByTestId('group-report-highlight')).toHaveLength(1);
    expect(getAllByTestId('group-report-concern')).toHaveLength(1);
    expect(getAllByTestId('group-report-recommendation')).toHaveLength(1);
    expect(getAllByTestId('group-health-flag-row')).toHaveLength(2);
    expect(getByText('Health flags (2)')).toBeTruthy();
    expect(getByText('no activity for 11 days')).toBeTruthy();
  });
});
