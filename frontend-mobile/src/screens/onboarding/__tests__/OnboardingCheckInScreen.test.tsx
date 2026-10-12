// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingCheckInScreen — how often the coach checks in on its own
// ABOUTME: A tap stores the cadence before the step is recorded; never is an answer, a skip stores nothing (carnet#831)

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { OnboardingCheckInScreen } from '../OnboardingCheckInScreen';
import { athleteApi, userApi } from '../../../services/api';
import { useOnboardingFlag } from '../../../hooks/useOnboardingFlag';

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useOnboardingFlag');
jest.mock('../../../hooks/useOnboardingProgress', () => ({
  useOnboardingProgress: () => [
    { id: 'check_in', labelKey: 'onboarding.stepCheckIn', status: 'current' },
  ],
}));
jest.mock('../../../services/api', () => ({
  athleteApi: {
    getCheckIn: jest.fn(),
    updateCheckIn: jest.fn(),
  },
  userApi: {
    setOnboardingStep: jest.fn(),
  },
}));

const mockMark = jest.fn();
const getCheckIn = athleteApi.getCheckIn as jest.Mock;
const updateCheckIn = athleteApi.updateCheckIn as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <OnboardingCheckInScreen />
    </QueryClientProvider>,
  );
}

describe('OnboardingCheckInScreen', () => {
  beforeEach(() => {
    mockMark.mockReset().mockResolvedValue(undefined);
    getCheckIn.mockReset().mockResolvedValue({ cadence: null });
    updateCheckIn.mockReset();
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    (useOnboardingFlag as jest.Mock).mockReturnValue({ done: false, mark: mockMark });
  });

  it('asks how often, with the four answers, under the progress hairline', () => {
    renderScreen();
    expect(screen.getByText('How often should your agent check in?')).toBeTruthy();
    expect(screen.getByText('Every morning')).toBeTruthy();
    expect(screen.getByText('Monday, Wednesday and Friday')).toBeTruthy();
    expect(screen.getByText('Every Monday')).toBeTruthy();
    expect(screen.getByText("Never — I'll start the conversation")).toBeTruthy();
    expect(screen.getByTestId('onboarding-progress-bar')).toBeTruthy();
  });

  it('stores the cadence tapped, then records the step and moves on', async () => {
    updateCheckIn.mockResolvedValue({ cadence: 'weekly' });
    renderScreen();

    fireEvent.press(screen.getByTestId('check-in-option-weekly'));

    await waitFor(() => expect(updateCheckIn).toHaveBeenCalledWith('weekly'));
    await waitFor(() => expect(setOnboardingStep).toHaveBeenCalledWith('check_in', 'complete'));
    expect(mockMark).toHaveBeenCalled();
  });

  it('takes never as an answer like the others', async () => {
    updateCheckIn.mockResolvedValue({ cadence: 'off' });
    renderScreen();

    fireEvent.press(screen.getByTestId('check-in-option-off'));

    await waitFor(() => expect(updateCheckIn).toHaveBeenCalledWith('off'));
    await waitFor(() => expect(mockMark).toHaveBeenCalled());
  });

  it('stays on the step and says so when the choice could not be stored', async () => {
    updateCheckIn.mockRejectedValue(new Error('offline'));
    renderScreen();

    fireEvent.press(screen.getByTestId('check-in-option-daily'));

    expect(await screen.findByText('Your check-in choice could not be saved. Try again.')).toBeTruthy();
    expect(setOnboardingStep).not.toHaveBeenCalled();
    expect(mockMark).not.toHaveBeenCalled();
  });

  it('a skip stores nothing, records the step skipped and moves on', async () => {
    renderScreen();

    fireEvent.press(screen.getByTestId('check-in-skip'));

    await waitFor(() => expect(mockMark).toHaveBeenCalled());
    expect(setOnboardingStep).toHaveBeenCalledWith('check_in', 'skipped');
    expect(updateCheckIn).not.toHaveBeenCalled();
  });
});
