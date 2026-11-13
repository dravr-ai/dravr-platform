// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingCoachGroupScreen — the athlete, the name and agent on one screen, then the invite
// ABOUTME: Pins create → thread → 30-day invite, the app.dravr.ai link, Share first with the QR behind a link, the seat card, retry and skip

import React from 'react';
import { render as rtlRender, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Share } from 'react-native';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { OnboardingCoachGroupScreen } from '../OnboardingCoachGroupScreen';
import { chatApi, coachesApi, groupsApi, userApi } from '../../../services/api';
import { useOnboardingFlag } from '../../../hooks/useOnboardingFlag';
import { useCoachGroupCreated } from '../../../hooks/useCoachGroupCreated';

const mockReplace = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ replace: mockReplace }),
}));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean', manages_roster: false } }),
}));
jest.mock('../../../hooks/useOnboardingFlag');
jest.mock('../../../hooks/useCoachGroupCreated');
jest.mock('../../../hooks/useOnboardingProgress', () => ({
  useOnboardingProgress: () => [
    { id: 'coach_group', labelKey: 'onboarding.stepGroup', status: 'current' },
  ],
}));
jest.mock('react-native-qrcode-svg', () => 'QRCode');
jest.mock('../../../services/api', () => ({
  userApi: {
    setOnboardingStep: jest.fn(),
    getCoachAccessRequest: jest.fn(),
    requestCoachAccess: jest.fn(),
  },
  groupsApi: { createGroup: jest.fn(), createInvite: jest.fn() },
  chatApi: { createConversation: jest.fn() },
  coachesApi: { list: jest.fn() },
}));

const mockMark = jest.fn();
const mockMarkCreated = jest.fn();
const createGroup = groupsApi.createGroup as jest.Mock;
const createInvite = groupsApi.createInvite as jest.Mock;
const createConversation = chatApi.createConversation as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;
const listAgents = coachesApi.list as jest.Mock;
const getCoachAccessRequest = userApi.getCoachAccessRequest as jest.Mock;
const requestCoachAccess = userApi.requestCoachAccess as jest.Mock;

const accessRequest = (status: 'pending' | 'granted' | 'declined') => ({
  id: 'r-1',
  user_id: 'u1',
  group_id: 'g-1',
  group_tenant_id: 't-1',
  status,
  created_at: '2026-10-07T12:00:00Z',
  decided_at: null,
  decided_by: null,
});

function render(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return rtlRender(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

const agent = (id: string, title: string, isHidden = false, tags: string[] = []) => ({
  id,
  title,
  description: null,
  category: 'training',
  is_hidden: isHidden,
  tags,
});

const group = (coachUserId: string | null) => ({
  id: 'g-1',
  name: 'Léa · Marathon',
  agent_id: 'a-2',
  coach_user_id: coachUserId,
});

/** The athlete and their goal; the group's name follows them. */
function describeTheAthlete() {
  fireEvent.changeText(screen.getByTestId('onboarding-group-athlete'), 'Léa');
  fireEvent.changeText(screen.getByTestId('onboarding-group-goal'), 'Marathon');
}

async function createTheGroup() {
  describeTheAthlete();
  await screen.findByTestId('onboarding-group-agent');
  fireEvent.press(screen.getByTestId('onboarding-group-create'));
}

describe('OnboardingCoachGroupScreen', () => {
  beforeEach(() => {
    mockMark.mockReset().mockResolvedValue(undefined);
    mockMarkCreated.mockReset();
    mockReplace.mockReset();
    createGroup.mockReset().mockResolvedValue(group('u1'));
    createConversation.mockReset().mockResolvedValue({ id: 'c-1' });
    createInvite.mockReset().mockResolvedValue({ code: 'ABCD2345' });
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    getCoachAccessRequest.mockReset().mockResolvedValue({ request: null });
    requestCoachAccess.mockReset();
    listAgents.mockReset().mockResolvedValue({
      agents: [
        agent('a-1', 'Road Running Agent', false, ['running']),
        agent('a-2', 'Endurance Agent', false, ['endurance']),
        agent('a-3', 'Hidden', true),
        agent('a-4', 'Roster Agent', false, ['coach-tool']),
      ],
      total: 4,
      metadata: {},
    });
    (useOnboardingFlag as jest.Mock).mockReturnValue({ done: false, mark: mockMark });
    (useCoachGroupCreated as jest.Mock).mockReturnValue({ created: false, mark: mockMarkCreated });
  });

  it('fills the group name in from the athlete and goal, and defaults the agent', async () => {
    render(<OnboardingCoachGroupScreen />);
    describeTheAthlete();
    expect(screen.getByTestId('onboarding-group-name').props.value).toBe('Léa · Marathon');
    expect(await screen.findByTestId('onboarding-group-agent')).toHaveTextContent(/Endurance Agent/);
    expect(screen.queryByTestId('onboarding-group-agents')).toBeNull();
  });

  it('offers the athlete-facing catalogue behind Change', async () => {
    render(<OnboardingCoachGroupScreen />);
    fireEvent.press(await screen.findByTestId('onboarding-group-agent-change'));

    expect(screen.getByTestId('onboarding-group-agent-a-1')).toBeTruthy();
    expect(screen.queryByTestId('onboarding-group-agent-a-3')).toBeNull();
    // A coach-facing agent never answers a group's athletes.
    expect(screen.queryByTestId('onboarding-group-agent-a-4')).toBeNull();
    fireEvent.press(screen.getByTestId('onboarding-group-agent-a-1'));
    expect(screen.getByTestId('onboarding-group-agent')).toHaveTextContent(/Road Running Agent/);
  });

  it('creates the group with its agent and goal, opens its thread and shows a 30-day invite link', async () => {
    render(<OnboardingCoachGroupScreen />);
    await createTheGroup();

    await waitFor(() =>
      expect(screen.getByTestId('onboarding-group-link')).toHaveTextContent(
        'https://app.dravr.ai/groups/join/ABCD2345',
      ),
    );
    expect(createGroup).toHaveBeenCalledWith({
      name: 'Léa · Marathon',
      agent_id: 'a-2',
      coach_is_me: true,
      description: 'Marathon',
    });
    expect(createConversation).toHaveBeenCalledWith({ group_id: 'g-1', agent_id: 'a-2' });
    expect(createInvite).toHaveBeenCalledWith('g-1', { expires_in_days: 30 });
    // The bar moves on to the invite.
    expect(mockMarkCreated).toHaveBeenCalled();
  });

  it('marks the cached chat list stale once the group thread exists, so the group is on it', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.setQueryData(QUERY_KEYS.chat.conversations(), { pages: [], pageParams: [] });
    rtlRender(
      <QueryClientProvider client={client}>
        <OnboardingCoachGroupScreen />
      </QueryClientProvider>,
    );
    await createTheGroup();

    await waitFor(() => expect(createConversation).toHaveBeenCalled());
    await waitFor(() =>
      expect(client.getQueryState(QUERY_KEYS.chat.conversations())?.isInvalidated).toBe(true),
    );
  });

  it('creates nothing without a name', async () => {
    render(<OnboardingCoachGroupScreen />);
    await screen.findByTestId('onboarding-group-agent');
    fireEvent.press(screen.getByTestId('onboarding-group-create'));
    expect(createGroup).not.toHaveBeenCalled();
  });

  it('says so when the agents cannot be loaded, and creates nothing', async () => {
    listAgents.mockRejectedValue(new Error('network'));
    render(<OnboardingCoachGroupScreen />);
    describeTheAthlete();

    await waitFor(() => expect(screen.getByText('Failed to load agents')).toBeTruthy());
    fireEvent.press(screen.getByTestId('onboarding-group-create'));
    expect(createGroup).not.toHaveBeenCalled();
  });

  it('the invite: Share first, the QR behind a link, named for the athlete', async () => {
    const share = jest.spyOn(Share, 'share').mockResolvedValue({ action: 'sharedAction' });
    render(<OnboardingCoachGroupScreen />);
    await createTheGroup();

    await waitFor(() => expect(screen.getByText('Invite Léa')).toBeTruthy());
    expect(screen.queryByTestId('onboarding-group-qr')).toBeNull();
    fireEvent.press(screen.getByTestId('onboarding-group-share'));
    expect(share).toHaveBeenCalledWith({
      message: 'Join Léa · Marathon on Dravr: https://app.dravr.ai/groups/join/ABCD2345',
    });
    fireEvent.press(screen.getByTestId('onboarding-group-qr-toggle'));
    expect(screen.getByTestId('onboarding-group-qr')).toBeTruthy();
    share.mockRestore();
  });

  it('says the coach coaches the group when the server made them its coach', async () => {
    render(<OnboardingCoachGroupScreen />);
    await createTheGroup();
    await waitFor(() => expect(screen.getByTestId('onboarding-group-seat-coach')).toBeTruthy());
    expect(screen.queryByTestId('coach-access-request')).toBeNull();
  });

  describe('the seat card for a coachless group (never "pending" before a request)', () => {
    async function createCoachless() {
      createGroup.mockResolvedValue(group(null));
      render(<OnboardingCoachGroupScreen />);
      await createTheGroup();
    }

    it('says not yet the coach, nothing asked yet, with the quiet request — and no mailto', async () => {
      await createCoachless();
      await waitFor(() => expect(screen.getByTestId('onboarding-group-seat-none')).toBeTruthy());
      expect(screen.getByText("You haven't asked for your seat yet.")).toBeTruthy();
      expect(screen.queryByText(/pending/i)).toBeNull();
      expect(screen.queryByText(/support@dravr\.ai/)).toBeNull();
    });

    it('asks in one tap, naming the group, then shows the dated request', async () => {
      requestCoachAccess.mockResolvedValue({ request: accessRequest('pending') });
      await createCoachless();
      await waitFor(() => expect(screen.getByTestId('coach-access-request')).toBeEnabled());
      fireEvent.press(screen.getByTestId('coach-access-request'));

      await waitFor(() => expect(screen.getByTestId('onboarding-group-seat-requested')).toBeTruthy());
      expect(requestCoachAccess).toHaveBeenCalledWith('g-1');
      expect(screen.getByText(/Request sent .*2026.* The Dravr team usually answers within 24 h\./)).toBeTruthy();
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
    });

    it('shows a request already waiting instead of the button', async () => {
      getCoachAccessRequest.mockResolvedValue({ request: accessRequest('pending') });
      await createCoachless();
      await waitFor(() => expect(screen.getByTestId('onboarding-group-seat-requested')).toBeTruthy());
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
    });

    it('offers the button again after a declined request', async () => {
      getCoachAccessRequest.mockResolvedValue({ request: accessRequest('declined') });
      await createCoachless();
      await waitFor(() => expect(screen.getByTestId('coach-access-declined')).toBeTruthy());
      expect(screen.getByTestId('coach-access-request')).toBeTruthy();
    });
  });

  it('retries after a failure without making a second group or thread', async () => {
    createInvite.mockRejectedValueOnce(new Error('network'));
    render(<OnboardingCoachGroupScreen />);
    await createTheGroup();

    await waitFor(() => expect(screen.getByText("We couldn't create the group. Try again.")).toBeTruthy());
    // The group exists now: it can no longer be put off.
    expect(screen.queryByTestId('onboarding-group-later')).toBeNull();
    fireEvent.press(screen.getByTestId('onboarding-group-create'));

    await waitFor(() => expect(screen.getByTestId('onboarding-group-link')).toBeTruthy());
    expect(createGroup).toHaveBeenCalledTimes(1);
    expect(createConversation).toHaveBeenCalledTimes(1);
  });

  it('completes the step from the invite, then opens the group thread', async () => {
    // The thread replaces the step only once the step is recorded.
    mockMark.mockImplementation(async () => {
      expect(mockReplace).not.toHaveBeenCalled();
    });
    render(<OnboardingCoachGroupScreen />);
    await createTheGroup();
    const done = await screen.findByTestId('onboarding-group-done');
    expect(screen.getByText("Go to Léa's group")).toBeTruthy();

    fireEvent.press(done);
    await waitFor(() =>
      expect(mockReplace).toHaveBeenCalledWith({
        pathname: '/(app)/chat/[conversationId]',
        params: { conversationId: 'c-1' },
      }),
    );
    expect(mockMark).toHaveBeenCalledTimes(1);
    expect(setOnboardingStep).toHaveBeenCalledWith('coach_group', 'complete');
  });

  it('puts the step off with Later and creates nothing', async () => {
    render(<OnboardingCoachGroupScreen />);
    fireEvent.press(screen.getByTestId('onboarding-group-later'));

    await waitFor(() => expect(mockMark).toHaveBeenCalled());
    expect(setOnboardingStep).toHaveBeenCalledWith('coach_group', 'skipped');
    expect(createGroup).not.toHaveBeenCalled();
    // Later leaves the landing to the layout (Home), never a thread.
    expect(mockReplace).not.toHaveBeenCalled();
  });
});
