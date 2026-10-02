// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingProfileTypeScreen — athlete / coach / both step
// ABOUTME: Verifies both coach choices persist coaching_persona=coach and only coach-only drops the athlete steps

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { OnboardingProfileTypeScreen } from '../OnboardingProfileTypeScreen';
import { userApi } from '../../../services/api';
import { useProfileTypeChosen } from '../../../hooks/useProfileTypeChosen';

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useProfileTypeChosen');
const mockMarkWaived = jest.fn();
jest.mock('../../../hooks/useOnboardingFlag', () => ({
  ATHLETE_STEPS_WAIVED_PREFIX: 'dravr.athlete_steps_waived.',
  useOnboardingFlag: () => ({ done: false, mark: mockMarkWaived }),
}));
jest.mock('../../../hooks/useOnboardingProgress', () => ({
  useOnboardingProgress: () => [
    { id: 'profile_type', labelKey: 'onboarding.stepAboutYou', status: 'current' },
  ],
}));
jest.mock('../../../services/api', () => ({
  userApi: {
    setCoachingPersona: jest.fn(),
    setOnboardingStep: jest.fn(),
  },
}));

const mockMarkChosen = jest.fn();
const setCoachingPersona = userApi.setCoachingPersona as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;

describe('OnboardingProfileTypeScreen', () => {
  beforeEach(() => {
    mockMarkChosen.mockClear();
    mockMarkWaived.mockClear();
    setCoachingPersona.mockReset().mockResolvedValue({});
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    (useProfileTypeChosen as jest.Mock).mockReturnValue({ markChosen: mockMarkChosen });
  });

  it('renders the athlete, coach and coach-who-trains choices, under the progress hairline', () => {
    render(<OnboardingProfileTypeScreen />);
    expect(screen.getByTestId('onboarding-progress-bar')).toBeTruthy();
    expect(screen.getByText("I'm an athlete")).toBeTruthy();
    expect(screen.getByText('I coach others')).toBeTruthy();
    expect(screen.getByText('I coach and I train')).toBeTruthy();
  });

  it('coach choice persists coaching_persona=coach, drops the athlete steps, then marks the step done', async () => {
    render(<OnboardingProfileTypeScreen />);
    fireEvent.press(screen.getByRole('button', { name: 'I coach others' }));
    await waitFor(() => expect(setCoachingPersona).toHaveBeenCalledWith('coach'));
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(mockMarkWaived).toHaveBeenCalled();
    expect(setOnboardingStep).toHaveBeenCalledWith('about_you', 'not_applicable');
    expect(setOnboardingStep).toHaveBeenCalledWith('parq', 'not_applicable');
  });

  it('coach-who-trains choice persists coaching_persona=coach and keeps the athlete steps', async () => {
    render(<OnboardingProfileTypeScreen />);
    fireEvent.press(screen.getByRole('button', { name: 'I coach and I train' }));
    await waitFor(() => expect(setCoachingPersona).toHaveBeenCalledWith('coach'));
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(mockMarkWaived).not.toHaveBeenCalled();
    expect(setOnboardingStep).not.toHaveBeenCalledWith('parq', 'not_applicable');
  });

  it('athlete choice marks the step done without writing a persona', async () => {
    render(<OnboardingProfileTypeScreen />);
    fireEvent.press(screen.getByRole('button', { name: "I'm an athlete" }));
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(setCoachingPersona).not.toHaveBeenCalled();
    expect(mockMarkWaived).not.toHaveBeenCalled();
  });

  it('stays on the step with an error when the persona write fails', async () => {
    setCoachingPersona.mockRejectedValueOnce(new Error('network'));
    render(<OnboardingProfileTypeScreen />);
    fireEvent.press(screen.getByRole('button', { name: 'I coach others' }));
    expect(await screen.findByTestId('profile-type-save-failed')).toBeTruthy();
    expect(mockMarkChosen).not.toHaveBeenCalled();
    expect(mockMarkWaived).not.toHaveBeenCalled();
  });
});
