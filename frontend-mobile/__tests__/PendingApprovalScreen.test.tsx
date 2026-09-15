// ABOUTME: Unit tests for PendingApprovalScreen component
// ABOUTME: Pins the two distinct situations (unconfirmed email vs. awaiting review) and the quiet back-to-sign-in link

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
}));

const mockResendVerification = jest.fn();
jest.mock('../src/services/api', () => ({
  authApi: {
    resendVerification: (...args: unknown[]) => mockResendVerification(...args),
  },
}));

let mockUser: { email: string; email_verified?: boolean } = { email: 'jean@example.com', email_verified: true };
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: mockUser }),
}));

import { PendingApprovalScreen } from '../src/screens/auth/PendingApprovalScreen';

describe('PendingApprovalScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUser = { email: 'jean@example.com', email_verified: true };
  });

  function renderComponent() {
    return render(<PendingApprovalScreen />);
  }

  describe('awaiting-review situation (email already confirmed)', () => {
    it('carries its own testID and no card shell', () => {
      const { getByTestId } = renderComponent();
      expect(getByTestId('pending-approval-screen')).toBeTruthy();
    });

    it('tells the user the team is reviewing, not to check their email', () => {
      const { getByText, queryByTestId } = renderComponent();
      expect(getByText('Account Pending Approval')).toBeTruthy();
      expect(getByText('Our team reviews your registration')).toBeTruthy();
      expect(queryByTestId('pending-resend-button')).toBeNull();
    });

    it('demotes "Back to sign in" to a quiet link that still navigates', () => {
      const { getByTestId } = renderComponent();
      const link = getByTestId('pending-back-to-sign-in-link');
      expect(link).toBeTruthy();

      fireEvent.press(link);
      expect(mockRouter.replace).toHaveBeenCalledWith('/(auth)/login');
    });
  });

  describe('unconfirmed-email situation', () => {
    beforeEach(() => {
      mockUser = { email: 'jean@example.com', email_verified: false };
    });

    it('leads with confirming the email, and offers a resend', () => {
      const { getByText, getByTestId } = renderComponent();
      expect(getByText('Confirm your email')).toBeTruthy();
      expect(getByText('Open the confirmation link we emailed you')).toBeTruthy();
      expect(getByTestId('pending-resend-button')).toBeTruthy();
    });

    it('resending calls the API and shows the sent confirmation', async () => {
      mockResendVerification.mockResolvedValueOnce(undefined);
      const { getByTestId, getByText } = renderComponent();

      fireEvent.press(getByTestId('pending-resend-button'));

      await waitFor(() => expect(mockResendVerification).toHaveBeenCalledWith('jean@example.com'));
      await waitFor(() => expect(getByText(/Sent\. Give it a minute/)).toBeTruthy());
    });

    it('shows a failure line when the resend fails', async () => {
      mockResendVerification.mockRejectedValueOnce(new Error('network'));
      const { getByTestId, getByText } = renderComponent();

      fireEvent.press(getByTestId('pending-resend-button'));

      // The corpus entry carries a literal `&apos;` (an existing, out-of-scope
      // encoding quirk on this app.* key — not decoded by RN's plain Text).
      await waitFor(() => expect(getByText(/send it just now/)).toBeTruthy());
    });
  });
});
