// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingCoachGroupScreen — a coach names a group and leaves with its invite
// ABOUTME: Pins create → thread → 30-day invite, the app.dravr.ai link, access pending, retry and skip

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { OnboardingCoachGroupScreen } from '../OnboardingCoachGroupScreen';
import { chatApi, groupsApi, userApi } from '../../../services/api';
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
}));

const mockMark = jest.fn();
const createGroup = groupsApi.createGroup as jest.Mock;
const createInvite = groupsApi.createInvite as jest.Mock;
const createConversation = chatApi.createConversation as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;

const group = (coachUserId: string | null) => ({
  id: 'g-1',
  name: 'Les Rouleurs',
  agent_id: 'a-1',
  coach_user_id: coachUserId,
});

function nameAndCreate() {
  fireEvent.changeText(screen.getByTestId('onboarding-group-name'), 'Les Rouleurs');
  fireEvent.press(screen.getByTestId('onboarding-group-create'));
}

describe('OnboardingCoachGroupScreen', () => {
  beforeEach(() => {
    mockMark.mockReset().mockResolvedValue(undefined);
    createGroup.mockReset().mockResolvedValue(group('u1'));
    createConversation.mockReset().mockResolvedValue({ id: 'c-1' });
    createInvite.mockReset().mockResolvedValue({ code: 'ABCD2345' });
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    (useOnboardingFlag as jest.Mock).mockReturnValue({ done: false, mark: mockMark });
  });

  it('creates the group as its coach, opens its thread and shows a 30-day invite link', async () => {
    render(<OnboardingCoachGroupScreen />);
    nameAndCreate();

    await waitFor(() =>
      expect(screen.getByTestId('onboarding-group-link')).toHaveTextContent(
        'https://app.dravr.ai/groups/join/ABCD2345',
      ),
    );
    expect(createGroup).toHaveBeenCalledWith({ name: 'Les Rouleurs', coach_is_me: true });
    expect(createConversation).toHaveBeenCalledWith({ group_id: 'g-1', agent_id: 'a-1' });
    expect(createInvite).toHaveBeenCalledWith('g-1', { expires_in_days: 30 });
    expect(screen.getByTestId('onboarding-group-qr')).toBeTruthy();
    expect(screen.queryByTestId('onboarding-group-access-pending')).toBeNull();
  });

  it('says coach access is pending when the group comes back without a coach', async () => {
    createGroup.mockResolvedValue(group(null));
    render(<OnboardingCoachGroupScreen />);
    nameAndCreate();

    await waitFor(() => expect(screen.getByTestId('onboarding-group-access-pending')).toBeTruthy());
  });

  it('retries after a failure without making a second group', async () => {
    createConversation.mockRejectedValueOnce(new Error('network'));
    render(<OnboardingCoachGroupScreen />);
    nameAndCreate();

    await waitFor(() => expect(screen.getByText("We couldn't create the group. Try again.")).toBeTruthy());
    fireEvent.press(screen.getByTestId('onboarding-group-create'));

    await waitFor(() => expect(screen.getByTestId('onboarding-group-link')).toBeTruthy());
    expect(createGroup).toHaveBeenCalledTimes(1);
  });

  it('completes the step from the share screen', async () => {
    render(<OnboardingCoachGroupScreen />);
    nameAndCreate();
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
