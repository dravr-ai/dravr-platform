// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingCoachGroupScreen — a coach names a group, picks its agent, leaves with its invite
// ABOUTME: Pins name → agent → create → thread → 30-day invite, the app.dravr.ai link, the coach-access request, retry and skip

import React from 'react';
import { render as rtlRender, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { OnboardingCoachGroupScreen } from '../OnboardingCoachGroupScreen';
import { chatApi, coachesApi, groupsApi, userApi } from '../../../services/api';
import { useOnboardingFlag } from '../../../hooks/useOnboardingFlag';

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useOnboardingFlag');
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
  name: 'Les Rouleurs',
  agent_id: 'a-2',
  coach_user_id: coachUserId,
});

function nameTheGroup() {
  fireEvent.changeText(screen.getByTestId('onboarding-group-name'), 'Les Rouleurs');
  fireEvent.press(screen.getByTestId('onboarding-group-next'));
}

async function nameAndCreate() {
  nameTheGroup();
  fireEvent.press(await screen.findByTestId('onboarding-group-agent-a-2'));
  fireEvent.press(screen.getByTestId('onboarding-group-create'));
}

describe('OnboardingCoachGroupScreen', () => {
  beforeEach(() => {
    mockMark.mockReset().mockResolvedValue(undefined);
    createGroup.mockReset().mockResolvedValue(group('u1'));
    createConversation.mockReset().mockResolvedValue({ id: 'c-1' });
    createInvite.mockReset().mockResolvedValue({ code: 'ABCD2345' });
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    getCoachAccessRequest.mockReset().mockResolvedValue({ request: null });
    requestCoachAccess.mockReset();
    listAgents.mockReset().mockResolvedValue({
      agents: [
        agent('a-1', 'Endurance Agent'),
        agent('a-2', 'Triathlon Agent'),
        agent('a-3', 'Hidden', true),
        agent('a-4', 'Roster Agent', false, ['coach-tool']),
      ],
      total: 4,
      metadata: {},
    });
    (useOnboardingFlag as jest.Mock).mockReturnValue({ done: false, mark: mockMark });
  });

  it('creates the group with the chosen agent, opens its thread and shows a 30-day invite link', async () => {
    render(<OnboardingCoachGroupScreen />);
    await nameAndCreate();

    await waitFor(() =>
      expect(screen.getByTestId('onboarding-group-link')).toHaveTextContent(
        'https://app.dravr.ai/groups/join/ABCD2345',
      ),
    );
    expect(createGroup).toHaveBeenCalledWith({
      name: 'Les Rouleurs',
      agent_id: 'a-2',
      coach_is_me: true,
    });
    expect(createConversation).toHaveBeenCalledWith({ group_id: 'g-1', agent_id: 'a-2' });
    expect(createInvite).toHaveBeenCalledWith('g-1', { expires_in_days: 30 });
    expect(screen.getByTestId('onboarding-group-qr')).toBeTruthy();
    expect(screen.queryByTestId('onboarding-group-access-pending')).toBeNull();
  });

  it('offers the visible athlete-facing catalogue and creates nothing until an agent is picked', async () => {
    render(<OnboardingCoachGroupScreen />);
    nameTheGroup();

    expect(await screen.findByTestId('onboarding-group-agent-a-1')).toBeTruthy();
    expect(screen.queryByTestId('onboarding-group-agent-a-3')).toBeNull();
    // A coach-facing agent never answers a group's athletes.
    expect(screen.queryByTestId('onboarding-group-agent-a-4')).toBeNull();
    fireEvent.press(screen.getByTestId('onboarding-group-create'));
    expect(createGroup).not.toHaveBeenCalled();
  });

  it('says so when the agents cannot be loaded, and creates nothing', async () => {
    listAgents.mockRejectedValue(new Error('network'));
    render(<OnboardingCoachGroupScreen />);
    nameTheGroup();

    await waitFor(() => expect(screen.getByText('Failed to load agents')).toBeTruthy());
    fireEvent.press(screen.getByTestId('onboarding-group-create'));
    expect(createGroup).not.toHaveBeenCalled();
  });

  it('goes back to the name keeping what was typed', async () => {
    render(<OnboardingCoachGroupScreen />);
    nameTheGroup();
    fireEvent.press(await screen.findByTestId('onboarding-group-back'));
    expect(screen.getByTestId('onboarding-group-name').props.value).toBe('Les Rouleurs');
  });

  it('says coach access is pending when the group comes back without a coach', async () => {
    createGroup.mockResolvedValue(group(null));
    render(<OnboardingCoachGroupScreen />);
    await nameAndCreate();

    await waitFor(() => expect(screen.getByTestId('onboarding-group-access-pending')).toBeTruthy());
  });

  describe('asking for coach access while it is pending (carnet#738)', () => {
    async function createCoachless() {
      createGroup.mockResolvedValue(group(null));
      render(<OnboardingCoachGroupScreen />);
      await nameAndCreate();
      await waitFor(() =>
        expect(screen.getByTestId('coach-access-request').props.accessibilityState?.disabled).not.toBe(
          true,
        ),
      );
    }

    it('no longer offers a mailto link', async () => {
      createGroup.mockResolvedValue(group(null));
      render(<OnboardingCoachGroupScreen />);
      await nameAndCreate();
      await waitFor(() => expect(screen.getByTestId('onboarding-group-access-pending')).toBeTruthy());
      expect(screen.queryByText(i18n.t('onboarding.groupAccessContact'))).toBeNull();
    });

    it('asks in one tap, naming the group, then says the request was sent', async () => {
      requestCoachAccess.mockResolvedValue({ request: accessRequest('pending') });
      await createCoachless();
      await waitFor(() => expect(screen.getByTestId('coach-access-request')).toBeEnabled());
      fireEvent.press(screen.getByTestId('coach-access-request'));

      await waitFor(() => expect(screen.getByTestId('coach-access-pending')).toBeTruthy());
      expect(requestCoachAccess).toHaveBeenCalledWith('g-1');
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
    });

    it('shows a request already waiting instead of the button', async () => {
      getCoachAccessRequest.mockResolvedValue({ request: accessRequest('pending') });
      createGroup.mockResolvedValue(group(null));
      render(<OnboardingCoachGroupScreen />);
      await nameAndCreate();

      await waitFor(() => expect(screen.getByTestId('coach-access-pending')).toBeTruthy());
      expect(screen.queryByTestId('coach-access-request')).toBeNull();
    });

    it('offers the button again after a declined request', async () => {
      getCoachAccessRequest.mockResolvedValue({ request: accessRequest('declined') });
      createGroup.mockResolvedValue(group(null));
      render(<OnboardingCoachGroupScreen />);
      await nameAndCreate();

      await waitFor(() => expect(screen.getByTestId('coach-access-declined')).toBeTruthy());
      expect(screen.getByTestId('coach-access-request')).toBeTruthy();
    });

    it('says so when the request fails, and retries', async () => {
      requestCoachAccess
        .mockRejectedValueOnce(new Error('network'))
        .mockResolvedValueOnce({ request: accessRequest('pending') });
      await createCoachless();
      await waitFor(() => expect(screen.getByTestId('coach-access-request')).toBeEnabled());
      fireEvent.press(screen.getByTestId('coach-access-request'));
      await waitFor(() => expect(screen.getByTestId('coach-access-failed')).toBeTruthy());

      fireEvent.press(screen.getByTestId('coach-access-request'));
      await waitFor(() => expect(screen.getByTestId('coach-access-pending')).toBeTruthy());
      expect(requestCoachAccess).toHaveBeenCalledTimes(2);
    });
  });

  it('retries after a failure without making a second group', async () => {
    createConversation.mockRejectedValueOnce(new Error('network'));
    render(<OnboardingCoachGroupScreen />);
    await nameAndCreate();

    await waitFor(() => expect(screen.getByText("We couldn't create the group. Try again.")).toBeTruthy());
    // The group exists now: its name and agent can no longer change.
    expect(screen.queryByTestId('onboarding-group-back')).toBeNull();
    fireEvent.press(screen.getByTestId('onboarding-group-create'));

    await waitFor(() => expect(screen.getByTestId('onboarding-group-link')).toBeTruthy());
    expect(createGroup).toHaveBeenCalledTimes(1);
  });

  it('completes the step from the share screen', async () => {
    render(<OnboardingCoachGroupScreen />);
    await nameAndCreate();
    await waitFor(() => expect(screen.getByText('Go to my group')).toBeTruthy());

    fireEvent.press(screen.getByText('Go to my group'));
    await waitFor(() => expect(mockMark).toHaveBeenCalled());
    expect(setOnboardingStep).toHaveBeenCalledWith('coach_group', 'complete');
  });

  it('puts the step off with Later and creates nothing', async () => {
    render(<OnboardingCoachGroupScreen />);
    fireEvent.press(screen.getByTestId('onboarding-group-later'));

    await waitFor(() => expect(mockMark).toHaveBeenCalled());
    expect(setOnboardingStep).toHaveBeenCalledWith('coach_group', 'skipped');
    expect(createGroup).not.toHaveBeenCalled();
  });
});
