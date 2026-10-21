// ABOUTME: Unit tests for PendingApprovalScreen component
// ABOUTME: Pins the two distinct situations (unconfirmed email vs. awaiting review) and the quiet back-to-sign-in link

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
let mockParams: { message?: string } = {};
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => mockParams,
}));

const mockResendVerification = jest.fn();
const mockClearImportTombstone = jest.fn();
jest.mock('../src/services/api', () => ({
  authApi: {
    resendVerification: (...args: unknown[]) => mockResendVerification(...args),
  },
  userApi: {
    clearImportTombstone: (...args: unknown[]) => mockClearImportTombstone(...args),
  },
}));

let mockUser: {
  email: string;
  email_verified?: boolean;
  email_tombstone?: { tombstone_id: string; deleted_at: string };
} = { email: 'jean@example.com', email_verified: true };
const mockUpdateUser = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: mockUser, updateUser: mockUpdateUser }),
}));

import { PendingApprovalScreen } from '../src/screens/auth/PendingApprovalScreen';

describe('PendingApprovalScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUser = { email: 'jean@example.com', email_verified: true };
    mockParams = {};
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

  describe('arriving from the register form', () => {
    it("shows the server's sentence rather than assuming a review", () => {
      mockParams = { message: 'Confirm your email to activate your account.' };
      const { getByTestId, queryByText } = renderComponent();
      expect(getByTestId('pending-register-message').props.children).toBe(
        'Confirm your email to activate your account.',
      );
      expect(queryByText(/Thank you for registering with Dravr/)).toBeNull();
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

      await waitFor(() =>
        expect(getByText('Couldn’t send it just now. Try again in a moment.')).toBeTruthy(),
      );
    });
  });

  describe('an email a deleted account left behind', () => {
    const tombstone = { tombstone_id: 'tomb-1', deleted_at: '2026-10-20T08:00:00+00:00' };

    it('says why the account waits and starts fresh on the athlete\'s tap', async () => {
      mockUser = { email: 'back@example.com', email_verified: true, email_tombstone: tombstone };
      mockClearImportTombstone.mockResolvedValue({ cleared: [], user_status: 'active' });
      const { getByText, getByTestId, queryByText } = renderComponent();

      expect(
        getByText(
          'This email belonged to a Dravr account deleted on Oct 20, 2026. Confirm you want to start fresh with it to continue.'
        )
      ).toBeTruthy();
      // Nobody reviews it, so the review steps are not promised.
      expect(queryByText('Our team reviews your registration')).toBeNull();

      fireEvent.press(getByTestId('pending-email-tombstoned-clear'));

      await waitFor(() =>
        expect(mockUpdateUser).toHaveBeenCalledWith({ user_status: 'active', email_tombstone: undefined })
      );
      expect(mockClearImportTombstone).toHaveBeenCalledWith('tomb-1');
    });

    it('asks for the address to be confirmed first, with no clear before it is', () => {
      mockUser = { email: 'back@example.com', email_verified: false, email_tombstone: tombstone };
      const { getByText, queryByTestId } = renderComponent();

      expect(getByText('Confirm your email address first, then start fresh.')).toBeTruthy();
      expect(queryByTestId('pending-email-tombstoned-clear')).toBeNull();
    });
  });
});
