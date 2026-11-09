// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingAnalyticsConsentScreen — the usage-analytics question for an unasked account
// ABOUTME: Yes and No are stored before the session's user moves on; a failed save stays to retry; no progress bar (carnet#1366)

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { ANALYTICS_POLICY_URL } from '@pierre/shared-constants';
import { OnboardingAnalyticsConsentScreen } from '../OnboardingAnalyticsConsentScreen';
import { userApi } from '../../../services/api';

const mockUpdateUser = jest.fn();
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1' }, updateUser: mockUpdateUser }),
}));
jest.mock('../../../services/api', () => ({
  userApi: {
    updateAnalyticsConsent: jest.fn(),
  },
}));
const mockOpenExternal = jest.fn();
jest.mock('../../../utils/openExternal', () => ({
  openExternal: (...args: unknown[]) => mockOpenExternal(...args),
}));

const updateAnalyticsConsent = userApi.updateAnalyticsConsent as jest.Mock;

describe('OnboardingAnalyticsConsentScreen', () => {
  beforeEach(() => {
    mockUpdateUser.mockReset().mockResolvedValue(undefined);
    mockOpenExternal.mockReset();
    updateAnalyticsConsent.mockReset().mockResolvedValue({ message: 'ok', enabled: true });
  });

  it('asks the question with both answers and no progress bar', () => {
    render(<OnboardingAnalyticsConsentScreen />);
    expect(screen.getByText('Help improve Dravr?')).toBeTruthy();
    expect(screen.getByText('Yes, share usage analytics')).toBeTruthy();
    expect(screen.getByText('No thanks')).toBeTruthy();
    expect(screen.queryByTestId('onboarding-progress-bar')).toBeNull();
  });

  it.each([
    ['analytics-consent-accept', true],
    ['analytics-consent-decline', false],
  ])('stores %s as %s, then marks the session user answered', async (testID, enabled) => {
    render(<OnboardingAnalyticsConsentScreen />);

    fireEvent.press(screen.getByTestId(testID));

    await waitFor(() =>
      expect(mockUpdateUser).toHaveBeenCalledWith({
        analytics_consent: enabled,
        analytics_consent_answered: true,
      }),
    );
    expect(updateAnalyticsConsent).toHaveBeenCalledWith(enabled);
  });

  it('keeps the person on the question when the answer is not saved', async () => {
    updateAnalyticsConsent.mockRejectedValueOnce(new Error('offline'));
    render(<OnboardingAnalyticsConsentScreen />);

    fireEvent.press(screen.getByTestId('analytics-consent-decline'));

    expect(await screen.findByText("Your answer wasn't saved. Try again.")).toBeTruthy();
    expect(mockUpdateUser).not.toHaveBeenCalled();
  });

  it("opens the privacy policy's usage-analytics section", () => {
    render(<OnboardingAnalyticsConsentScreen />);
    fireEvent.press(screen.getByTestId('analytics-consent-policy'));
    expect(mockOpenExternal).toHaveBeenCalledWith(ANALYTICS_POLICY_URL, expect.any(Function));
  });
});
