// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingProfileTypeScreen — athlete-vs-coach step
// ABOUTME: Verifies the coach choice persists coaching_persona=coach and both choices mark the step done

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { OnboardingProfileTypeScreen } from '../OnboardingProfileTypeScreen';
import { userApi } from '../../../services/api';
import { useProfileTypeChosen } from '../../../hooks/useProfileTypeChosen';

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useProfileTypeChosen');
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
    setCoachingPersona.mockReset().mockResolvedValue({});
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    (useProfileTypeChosen as jest.Mock).mockReturnValue({ markChosen: mockMarkChosen });
  });

  it('renders both the athlete and coach choices', () => {
    render(<OnboardingProfileTypeScreen />);
    expect(screen.getByText("I'm an athlete")).toBeTruthy();
    expect(screen.getByText('I coach others')).toBeTruthy();
  });

  it('coach choice persists coaching_persona=coach then marks the step done', async () => {
    render(<OnboardingProfileTypeScreen />);
    fireEvent.press(screen.getByRole('button', { name: 'I coach others' }));
    await waitFor(() => expect(setCoachingPersona).toHaveBeenCalledWith('coach'));
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
  });

  it('athlete choice marks the step done without writing a persona', async () => {
    render(<OnboardingProfileTypeScreen />);
    fireEvent.press(screen.getByRole('button', { name: "I'm an athlete" }));
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(setCoachingPersona).not.toHaveBeenCalled();
  });
});
