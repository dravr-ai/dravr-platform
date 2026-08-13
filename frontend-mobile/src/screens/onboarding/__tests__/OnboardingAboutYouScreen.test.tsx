// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingAboutYouScreen — the three questions before the provider gate
// ABOUTME: Asserts the answers actually reach the API, since a step that silently persists nothing is the bug this replaced

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { OnboardingAboutYouScreen } from '../OnboardingAboutYouScreen';
import { userApi } from '../../../services/api';
import { useOnboardingFlag } from '../../../hooks/useOnboardingFlag';

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useOnboardingFlag');
jest.mock('../../../services/api', () => ({
  userApi: {
    saveAboutYou: jest.fn(),
    setOnboardingStep: jest.fn(),
  },
}));

const mockMark = jest.fn();
const saveAboutYou = userApi.saveAboutYou as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;

describe('OnboardingAboutYouScreen', () => {
  beforeEach(() => {
    mockMark.mockClear();
    saveAboutYou.mockReset().mockResolvedValue({ facts_written: 3 });
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    (useOnboardingFlag as jest.Mock).mockReturnValue({ done: false, mark: mockMark });
  });

  it('offers the sport choices and both free-text questions', () => {
    render(<OnboardingAboutYouScreen />);
    expect(screen.getByText('Running')).toBeTruthy();
    expect(screen.getByText('Cycling')).toBeTruthy();
    expect(screen.getByText('What are you working toward?')).toBeTruthy();
    expect(screen.getByText('And why does it matter to you?')).toBeTruthy();
  });

  it('sends the chosen sport, goal and North Star to the API', async () => {
    render(<OnboardingAboutYouScreen />);

    fireEvent.press(screen.getByText('Running'));
    fireEvent.changeText(
      screen.getByPlaceholderText('A first half-marathon in the spring, say'),
      'Sub-40 10k',
    );
    fireEvent.changeText(
      screen.getByPlaceholderText('Keeping up with my kids on the trail'),
      'Still running trails at 70',
    );
    fireEvent.press(screen.getByText('Continue'));

    await waitFor(() =>
      expect(saveAboutYou).toHaveBeenCalledWith({
        primary_sport: 'Running',
        goal: 'Sub-40 10k',
        north_star: 'Still running trails at 70',
      }),
    );
    await waitFor(() => expect(setOnboardingStep).toHaveBeenCalledWith('about_you', 'complete'));
    await waitFor(() => expect(mockMark).toHaveBeenCalled());
  });

  it('skipping records the step as skipped and never calls the API', async () => {
    render(<OnboardingAboutYouScreen />);
    fireEvent.press(screen.getByText('Skip for now'));

    await waitFor(() => expect(setOnboardingStep).toHaveBeenCalledWith('about_you', 'skipped'));
    expect(saveAboutYou).not.toHaveBeenCalled();
    await waitFor(() => expect(mockMark).toHaveBeenCalled());
  });

  it('a failed save still advances — the answers are a head start, not a gate', async () => {
    saveAboutYou.mockRejectedValue(new Error('network'));
    render(<OnboardingAboutYouScreen />);

    fireEvent.press(screen.getByText('Cycling'));
    fireEvent.press(screen.getByText('Continue'));

    await waitFor(() => expect(mockMark).toHaveBeenCalled());
  });
});
