// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingParqScreen — the pre-participation medical screen
// ABOUTME: Pins that a "yes" is submitted rather than dropped, and that the screen never blocks sign-up

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { OnboardingParqScreen } from '../OnboardingParqScreen';
import { userApi } from '../../../services/api';
import { useOnboardingFlag } from '../../../hooks/useOnboardingFlag';

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useOnboardingFlag');
jest.mock('../../../hooks/useOnboardingProgress', () => ({
  useOnboardingProgress: () => [
    { id: 'parq', labelKey: 'onboarding.stepHealthCheck', status: 'current' },
  ],
}));
jest.mock('../../../services/api', () => ({
  userApi: {
    getParqQuestions: jest.fn(),
    submitParq: jest.fn(),
    setOnboardingStep: jest.fn(),
  },
}));

const mockMark = jest.fn();
const getParqQuestions = userApi.getParqQuestions as jest.Mock;
const submitParq = userApi.submitParq as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;

const QUESTIONS = [
  { id: 'heart_condition', text: 'Has a doctor ever said that you have a heart condition?' },
  { id: 'joint_problem', text: 'Do you have a bone, joint, or soft-tissue problem?' },
];

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <OnboardingParqScreen />
    </QueryClientProvider>,
  );
}

describe('OnboardingParqScreen', () => {
  beforeEach(() => {
    mockMark.mockClear();
    getParqQuestions.mockReset().mockResolvedValue({ questions: QUESTIONS });
    submitParq.mockReset().mockResolvedValue({ flags_raised: 1 });
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    (useOnboardingFlag as jest.Mock).mockReturnValue({ done: false, mark: mockMark });
  });

  it('renders every question the server returns, under the progress hairline', async () => {
    renderScreen();
    await waitFor(() => expect(screen.getByText(QUESTIONS[0].text)).toBeTruthy());
    expect(screen.getByText(QUESTIONS[1].text)).toBeTruthy();
    expect(screen.getByTestId('onboarding-progress-bar')).toBeTruthy();
  });

  it('marks the chosen answer with a trailing check', async () => {
    renderScreen();
    await waitFor(() => expect(screen.getByText(QUESTIONS[0].text)).toBeTruthy());

    const yesRow = screen.getByTestId('parq-heart_condition-yes');
    expect(yesRow.props.accessibilityState).toEqual({ selected: false });
    fireEvent.press(screen.getAllByText('Yes')[0]);
    expect(screen.getByTestId('parq-heart_condition-yes').props.accessibilityState).toEqual({ selected: true });
  });

  it('submits a yes as yes — the flag is the whole point of the screen', async () => {
    renderScreen();
    await waitFor(() => expect(screen.getByText(QUESTIONS[0].text)).toBeTruthy());

    // First question yes, second no.
    fireEvent.press(screen.getAllByText('Yes')[0]);
    fireEvent.press(screen.getAllByText('No')[1]);
    fireEvent.press(screen.getByText('Continue'));

    await waitFor(() =>
      expect(submitParq).toHaveBeenCalledWith([
        { id: 'heart_condition', yes: true },
        { id: 'joint_problem', yes: false },
      ]),
    );
    await waitFor(() => expect(setOnboardingStep).toHaveBeenCalledWith('parq', 'complete'));
  });

  it('cannot continue until every question is answered', async () => {
    renderScreen();
    await waitFor(() => expect(screen.getByText(QUESTIONS[0].text)).toBeTruthy());

    expect(screen.getByText('Answer all to continue')).toBeTruthy();
    fireEvent.press(screen.getAllByText('No')[0]);
    expect(screen.getByText('Answer all to continue')).toBeTruthy();

    fireEvent.press(screen.getAllByText('No')[1]);
    await waitFor(() => expect(screen.getByText('Continue')).toBeTruthy());
  });

  it('a failed submit still advances — a health screen must never block sign-up', async () => {
    submitParq.mockRejectedValue(new Error('network'));
    renderScreen();
    await waitFor(() => expect(screen.getByText(QUESTIONS[0].text)).toBeTruthy());

    fireEvent.press(screen.getAllByText('No')[0]);
    fireEvent.press(screen.getAllByText('No')[1]);
    fireEvent.press(screen.getByText('Continue'));

    await waitFor(() => expect(mockMark).toHaveBeenCalled());
  });
});
