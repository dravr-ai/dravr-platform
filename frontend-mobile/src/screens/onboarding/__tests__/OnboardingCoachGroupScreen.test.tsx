// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingCoachGroupScreen — a coach names a group, picks its agent, leaves with its invite
// ABOUTME: Pins name → agent → create → thread → 30-day invite, the app.dravr.ai link, access pending, retry and skip

import React from 'react';
import { render as rtlRender, screen, fireEvent, waitFor } from '@testing-library/react-native';
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
  userApi: { setOnboardingStep: jest.fn() },
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

function render(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return rtlRender(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

const agent = (id: string, title: string, isHidden = false) => ({
  id,
  title,
  description: null,
  category: 'training',
  is_hidden: isHidden,
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
    listAgents.mockReset().mockResolvedValue({
      agents: [agent('a-1', 'Endurance Agent'), agent('a-2', 'Triathlon Agent'), agent('a-3', 'Hidden', true)],
      total: 3,
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

  it('offers the visible catalogue and creates nothing until an agent is picked', async () => {
    render(<OnboardingCoachGroupScreen />);
    nameTheGroup();

    expect(await screen.findByTestId('onboarding-group-agent-a-1')).toBeTruthy();
    expect(screen.queryByTestId('onboarding-group-agent-a-3')).toBeNull();
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
