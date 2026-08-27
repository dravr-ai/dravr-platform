// ABOUTME: carnet #55/#52/#53/#60 e2e — Group info inside the group's chat: admin controls, consent, report
// ABOUTME: Asserts the consent PUT carries the real value and that every control reaches the same routes as before

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert, Share } from 'react-native';
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
  useLocalSearchParams: () => ({ conversationId: 'conv-group-1' }),
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

import { GroupInfoSheet } from '../../src/screens/groups/GroupInfoSheet';

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

const onClose = jest.fn();
const onLeft = jest.fn();

/**
 * Group info as the chat header opens it: the sheet's content, for the group
 * the open thread is scoped to. Sections other than Members start collapsed,
 * so a test opens the one it is about.
 */
function renderGroup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <GroupInfoSheet groupId="group-1" fallbackName="Harricana 2027" onClose={onClose} onLeft={onLeft} />
    </QueryClientProvider>,
  );
}

describe('carnet #55/#52 — Group info admin controls + peer consent', () => {
  let stub: HttpStub;
  let ownerConsent: boolean;
  let shareMock: jest.Mock;

  beforeEach(() => {
    ownerConsent = false;
    mockBack.mockClear();
    mockPush.mockClear();
    onClose.mockClear();
    onLeft.mockClear();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    shareMock = jest
      .spyOn(Share, 'share')
      .mockResolvedValue({ action: 'sharedAction' } as never) as unknown as jest.Mock;
    stub = installHttpStub({
      'GET /api/groups/group-1': { data: GROUP },
      'GET /api/groups/group-1/members': () => ({ data: membersResponse(ownerConsent) }),
      'GET /api/groups/group-1/stats': { data: STATS },
      'GET /api/groups/group-1/invites': { data: INVITES },
      'GET /api/groups/permissions': { data: PERMISSIONS },
      'GET /api/groups/group-1/report': { data: REPORT },
      'GET /api/groups/group-1/health': { data: HEALTH },
      'GET /api/groups/group-1/transcript': { data: { group_id: 'group-1', entries: [] } },
      'GET /api/coaches': { data: { coaches: [] } },
      'PUT /api/groups/group-1/members/me/consent': (request) => {
        ownerConsent = (request.body as { consent: boolean }).consent;
        return { data: { success: true } };
      },
      'PUT /api/groups/group-1/members/user-phil/role': { data: { success: true } },
      'DELETE /api/groups/group-1/invites/invite-1': { data: { success: true } },
      'POST /api/groups/group-1/invites': {
        status: 201,
        data: { ...INVITES.invites[0], id: 'invite-2', code: 'HARRI-NEW', use_count: 0 },
      },
    });
  });

  afterEach(() => {
    stub.restore();
    jest.restoreAllMocks();
  });

  it('binds the consent switch to the caller row and writes the real value', async () => {
    const { getByTestId, findByTestId } = renderGroup();

    fireEvent.press(await findByTestId('group-info-settings-toggle'));
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

  it('lists live invites in Group info and deactivates one', async () => {
    const { getByTestId, getByText, findByTestId } = renderGroup();

    await act(async () => {
      fireEvent.press(await findByTestId('group-info-invites-toggle'));
    });

    await waitFor(() => {
      expect(getByTestId('group-invite-invite-1')).toBeTruthy();
    });
    expect(getByText('HARRI-7X2')).toBeTruthy();
    expect(getByText('Member invite · used 3×')).toBeTruthy();
    expect(getByText('Invites (1)')).toBeTruthy();

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
      'GET /api/groups/group-1/transcript': { data: { group_id: 'group-1', entries: [] } },
      'GET /api/coaches': { data: { coaches: [] } },
      'PUT /api/groups/group-1': { data: { ...GROUP, respond_mode: 'mentions' } },
    });

    const { getByTestId, findByTestId } = renderGroup();

    await act(async () => {
      fireEvent.press(await findByTestId('group-info-settings-toggle'));
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

  // The invite is created from inside the group's chat now — there is no
  // Groups tab to open first — and it is the code the `/group join` command
  // and the invite link both carry.
  it('creates a member invite from Group info and shares its code', async () => {
    const { getByTestId, findByTestId } = renderGroup();

    await act(async () => {
      fireEvent.press(await findByTestId('group-info-invites-toggle'));
    });
    await act(async () => {
      fireEvent.press(getByTestId('group-info-create-invite'));
    });

    const prompt = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(prompt[0]).toBe('Create Invite');
    await act(async () => {
      prompt[2].find((button) => button.text === 'Member (athlete)')?.onPress?.();
    });

    await waitFor(() => {
      const createRequest = stub
        .requestsFor('POST')
        .find((request) => request.url === '/api/groups/group-1/invites');
      expect(createRequest?.body).toEqual({ expires_in_days: 7 });
    });
    await waitFor(() => expect(shareMock).toHaveBeenCalled());
    expect((shareMock.mock.calls.at(-1)?.[0] as { message: string }).message).toContain('HARRI-NEW');
  });

  it('renders the weekly report and one row per flagged member', async () => {
    const { getByTestId, getAllByTestId, getByText, findByTestId } = renderGroup();

    await act(async () => {
      fireEvent.press(await findByTestId('group-info-analytics-toggle'));
    });
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
