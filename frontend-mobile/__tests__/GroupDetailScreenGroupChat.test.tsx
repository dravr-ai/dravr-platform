// ABOUTME: Tests GroupDetailScreen's group-scoped chat, consent switch, and insights section
// ABOUTME: Pins that group_id reaches createConversation and the caller's OWN row drives consent

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({ groupId: 'group-1' }),
}));

const CALLER_ID = 'a1f4d8d9-7498-4881-82c3-4c5c13161561';
const OTHER_ID = '7c2b6b64-9f7e-4f7f-9d0a-1d3f5b2a8c11';

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    isAuthenticated: true,
    user: { id: CALLER_ID, user_id: CALLER_ID, email: 'caller@dravr.ai' },
  }),
}));

const mockUpdateConsent = jest.fn();
const mockWeeklyDigest = jest.fn();
const mockReport = jest.fn();
const mockFlags = jest.fn();

jest.mock('../src/hooks/useGroups', () => ({
  useGroup: () => ({
    group: {
      id: 'group-1',
      name: 'Marathon Squad',
      description: null,
      coach_id: 'coach-1',
      owner_id: CALLER_ID,
      peer_data_sharing: true,
      respond_mode: 'all',
      is_active: true,
    },
    isLoading: false,
    isError: false,
    error: null,
    refetch: jest.fn(),
  }),
  useGroupMembers: () => ({
    members: [
      // The other member is listed first on purpose: binding the switch to the
      // first active row hands the caller somebody else's consent.
      {
        id: 'membership-2',
        group_id: 'group-1',
        user_id: OTHER_ID,
        role: 'member',
        peer_sharing_consent: true,
        joined_at: '2026-08-24T19:46:51.548903+00:00',
        display_name: 'phil@dravr.ai',
      },
      {
        id: 'membership-1',
        group_id: 'group-1',
        user_id: CALLER_ID,
        role: 'owner',
        peer_sharing_consent: false,
        joined_at: '2026-08-24T19:46:51.548903+00:00',
        display_name: 'caller@dravr.ai',
      },
    ],
    isLoading: false,
    isError: false,
    error: null,
    refetch: jest.fn(),
  }),
  useGroupStats: () => ({ stats: null, isLoading: false, isError: false, error: null, refetch: jest.fn() }),
  useGroupActions: () => ({ leaveGroup: jest.fn(), isLeaving: false }),
  useGroupInvites: () => ({ invites: [], isLoading: false, refetch: jest.fn() }),
  useUpdateGroup: () => ({ updateGroup: jest.fn(), isPending: false }),
  useDeleteGroup: () => ({ deleteGroup: jest.fn(), isPending: false }),
  useDeactivateInvite: () => ({ deactivateInvite: jest.fn(), isPending: false }),
  useUpdateMemberRole: () => ({ updateRole: jest.fn(), isPending: false }),
  useUpdatePeerConsent: () => ({ updateConsent: mockUpdateConsent, isPending: false }),
  useGroupPermissions: () => ({ canCreate: true, policy: 'everyone', weeklyDigest: mockWeeklyDigest() }),
  useGroupWeeklyReport: () => ({ report: mockReport(), isLoading: false, refetch: jest.fn() }),
  useGroupTranscript: () => ({ transcript: { group_id: 'group-1', members: [], entries: [] }, isLoading: false, isError: false, refetch: jest.fn() }),
  useGroupHealthFlags: () => {
    const flags = mockFlags();
    return { flags, total: flags.length, isLoading: false, refetch: jest.fn() };
  },
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
import { chatApi } from '../src/services/api';

const REPORT = {
  summary:
    'Marathon Squad had 2/2 active members this week with average volume of 41.5km. Overall trend: stable.',
  highlights: ['caller@dravr.ai is in fresh form (TSB +9, 12% of CTL)'],
  concerns: ['phil@dravr.ai: no activity for 11 days'],
  recommendations: ['Review 1 flagged member(s) and consider recovery adjustments.'],
  stats: {
    total_members: 2,
    active_members: 2,
    avg_weekly_volume_km: 41.5,
    avg_ctl: 55,
    flagged_members: 1,
    weekly_trend: 'stable',
  },
};

const FLAGS = [
  {
    user_id: OTHER_ID,
    display_name: 'phil@dravr.ai',
    flag_type: 'inactive',
    severity: 'warning',
    detail: 'no activity for 11 days',
  },
  {
    user_id: CALLER_ID,
    display_name: 'caller@dravr.ai',
    flag_type: 'volume_drop',
    severity: 'warning',
    detail: 'weekly volume down 35%',
  },
];

describe('GroupDetailScreen group chat and insights', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUpdateConsent.mockResolvedValue(undefined);
    mockWeeklyDigest.mockReturnValue(true);
    mockReport.mockReturnValue(REPORT);
    mockFlags.mockReturnValue(FLAGS);
  });

  it('opens a group-scoped conversation titled with the group name', async () => {
    (chatApi.createConversation as jest.Mock).mockResolvedValue({
      id: 'conv-9',
      title: 'Marathon Squad',
      group_id: 'group-1',
    });

    const { getByTestId } = render(<GroupDetailScreen />);
    fireEvent.press(getByTestId('chat-with-coach-button'));

    await waitFor(() => {
      expect(chatApi.createConversation).toHaveBeenCalledWith({
        title: 'Marathon Squad',
        coach_id: 'coach-1',
        group_id: 'group-1',
      });
    });
    await waitFor(() => {
      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: '/(app)/(tabs)/(chat)',
        params: { conversationId: 'conv-9' },
      });
    });
  });

  it('binds the consent switch to the caller own row and sends the new value', async () => {
    const { getByTestId } = render(<GroupDetailScreen />);

    const toggle = getByTestId('peer-consent-switch');
    // The caller's own row has consent off; the other member's row has it on.
    expect(toggle.props.value).toBe(false);

    fireEvent(toggle, 'valueChange', true);

    await waitFor(() => {
      expect(mockUpdateConsent).toHaveBeenCalledWith(true);
    });
    expect(mockUpdateConsent).toHaveBeenCalledTimes(1);
  });

  it('renders the weekly report and one row per flagged member', async () => {
    const { getByTestId, getAllByTestId, getByText } = render(<GroupDetailScreen />);

    await waitFor(() => {
      expect(getByTestId('group-report-summary').props.children).toBe(REPORT.summary);
    });
    expect(getAllByTestId('group-report-highlight')).toHaveLength(1);
    expect(getAllByTestId('group-report-concern')).toHaveLength(1);
    expect(getAllByTestId('group-report-recommendation')).toHaveLength(1);
    expect(getAllByTestId('group-health-flag-row')).toHaveLength(2);
    expect(getByText('Health flags (2)')).toBeTruthy();
  });

  it('withholds the report when the tenant tier does not enable the weekly digest', async () => {
    mockWeeklyDigest.mockReturnValue(false);

    const { getByTestId, queryByTestId } = render(<GroupDetailScreen />);

    await waitFor(() => {
      expect(getByTestId('group-insights-tier-locked')).toBeTruthy();
    });
    expect(queryByTestId('group-report-summary')).toBeNull();
    expect(queryByTestId('group-health-flag-row')).toBeNull();
  });
});
