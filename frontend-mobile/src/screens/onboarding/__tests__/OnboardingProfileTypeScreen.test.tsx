// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingProfileTypeScreen — athlete / coach / both step
// ABOUTME: A tap selects and Continue records the coaching role (never the persona); only coach-only drops the athlete steps

import React from 'react';
import { render as rtlRender, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { OnboardingProfileTypeScreen } from '../OnboardingProfileTypeScreen';
import { QUERY_KEYS } from '@pierre/shared-constants';
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
    { id: 'profile_type', labelKey: 'onboarding.stepProfile', status: 'current' },
  ],
}));
jest.mock('../../../services/api', () => ({
  userApi: {
    setCoachingRole: jest.fn(),
    setOnboardingStep: jest.fn(),
  },
}));

const mockMarkChosen = jest.fn();
const setCoachingRole = userApi.setCoachingRole as jest.Mock;
const setOnboardingStep = userApi.setOnboardingStep as jest.Mock;

/** The screen writes into the cached onboarding status, so it renders under a query client. */
function render(client = new QueryClient({ defaultOptions: { queries: { retry: false } } })) {
  return rtlRender(
    <QueryClientProvider client={client}>
      <OnboardingProfileTypeScreen />
    </QueryClientProvider>,
  );
}

/** Select a choice, then press Continue. */
function choose(name: string) {
  fireEvent.press(screen.getByRole('radio', { name }));
  fireEvent.press(screen.getByTestId('profile-type-continue'));
}

describe('OnboardingProfileTypeScreen', () => {
  beforeEach(() => {
    mockMarkChosen.mockClear();
    mockMarkWaived.mockClear();
    setCoachingRole.mockReset().mockResolvedValue({});
    setOnboardingStep.mockReset().mockResolvedValue(undefined);
    (useProfileTypeChosen as jest.Mock).mockReturnValue({ markChosen: mockMarkChosen });
  });

  it('renders the athlete, coach and coach-who-trains choices, under the progress hairline', () => {
    render();
    expect(screen.getByTestId('onboarding-progress-bar')).toBeTruthy();
    expect(screen.getByText("I'm an athlete")).toBeTruthy();
    expect(screen.getByText('I coach others')).toBeTruthy();
    expect(screen.getByText('I coach and I train')).toBeTruthy();
    expect(
      screen.getByText("Dravr reads your athletes' training, drafts plans you publish, and answers with you in their group."),
    ).toBeTruthy();
  });

  it('a tap only selects: nothing is recorded until Continue, and another tap changes the choice', async () => {
    render();
    expect(screen.getByTestId('profile-type-continue')).toBeDisabled();
    fireEvent.press(screen.getByRole('radio', { name: 'I coach others' }));
    expect(screen.getByRole('radio', { name: 'I coach others' })).toBeSelected();
    expect(setCoachingRole).not.toHaveBeenCalled();

    choose("I'm an athlete");
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(setCoachingRole).toHaveBeenCalledTimes(1);
    expect(setCoachingRole).toHaveBeenCalledWith(false);
  });

  it('coach choice records the coach role, drops the athlete steps, then marks the step done', async () => {
    render();
    choose('I coach others');
    await waitFor(() => expect(setCoachingRole).toHaveBeenCalledWith(true));
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(mockMarkWaived).toHaveBeenCalled();
    expect(setOnboardingStep).toHaveBeenCalledWith('about_you', 'not_applicable');
    expect(setOnboardingStep).toHaveBeenCalledWith('parq', 'not_applicable');
  });

  it('coach-who-trains choice records the coach role and keeps the athlete steps', async () => {
    render();
    choose('I coach and I train');
    await waitFor(() => expect(setCoachingRole).toHaveBeenCalledWith(true));
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(mockMarkWaived).not.toHaveBeenCalled();
    expect(setOnboardingStep).not.toHaveBeenCalledWith('parq', 'not_applicable');
  });

  it('athlete choice records the role as off and marks the step done', async () => {
    render();
    choose("I'm an athlete");
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(setCoachingRole).toHaveBeenCalledWith(false);
    expect(mockMarkWaived).not.toHaveBeenCalled();
  });

  it('stays on the step with an error when the role write fails', async () => {
    setCoachingRole.mockRejectedValueOnce(new Error('network'));
    render();
    choose('I coach others');
    expect(await screen.findByTestId('profile-type-save-failed')).toBeTruthy();
    expect(mockMarkChosen).not.toHaveBeenCalled();
    expect(mockMarkWaived).not.toHaveBeenCalled();
  });

  it('writes the chosen role into the cached onboarding status, so the next bar draws its journey', async () => {
    const client = new QueryClient();
    client.setQueryData(QUERY_KEYS.user.onboardingStatus(), { coaches_others: false, steps: [] });
    render(client);
    choose('I coach and I train');
    await waitFor(() => expect(mockMarkChosen).toHaveBeenCalled());
    expect(client.getQueryData(QUERY_KEYS.user.onboardingStatus())).toMatchObject({ coaches_others: true });
  });
});
