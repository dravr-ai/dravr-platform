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
const mockRedeemSignupCode = jest.fn();
jest.mock('../src/services/api', () => ({
  authApi: {
    resendVerification: (...args: unknown[]) => mockResendVerification(...args),
  },
  userApi: {
    clearImportTombstone: (...args: unknown[]) => mockClearImportTombstone(...args),
    redeemSignupCode: (...args: unknown[]) => mockRedeemSignupCode(...args),
  },
}));

let mockUser: null | {
  email: string;
  email_verified?: boolean;
  email_tombstone?: { tombstone_id: string; deleted_at: string };
} = { email: 'jean@example.com', email_verified: true };
const mockUpdateUser = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: mockUser, updateUser: mockUpdateUser }),
}));

import { PendingApprovalScreen } from '../src/screens/auth/PendingApprovalScreen';
import { apiRefusal } from '../integration/app/helpers/apiRefusal';

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

  // A code an admin or a coach handed out approves the waiting account
  // (carnet#1128).
  describe('signup code', () => {
    function enterCode(view: ReturnType<typeof renderComponent>, code: string) {
      fireEvent.changeText(view.getByTestId('pending-signup-code-input'), code);
      fireEvent.press(view.getByTestId('pending-signup-code-apply'));
    }

    it('is not offered before anyone is signed in', () => {
      mockUser = null;
      mockParams = { message: 'Registration received.' };
      const { queryByTestId } = renderComponent();
      expect(queryByTestId('pending-signup-code')).toBeNull();
    });

    it('is not offered beside a deleted account\'s address', () => {
      mockUser = {
        email: 'back@example.com',
        email_verified: true,
        email_tombstone: { tombstone_id: 'tomb-1', deleted_at: '2026-10-20T08:00:00+00:00' },
      };
      const { queryByTestId } = renderComponent();
      expect(queryByTestId('pending-signup-code')).toBeNull();
    });

    it('holds the button until a code is typed', () => {
      const { getByTestId } = renderComponent();
      expect(getByTestId('pending-signup-code-apply')).toBeDisabled();
      fireEvent.changeText(getByTestId('pending-signup-code-input'), '   ');
      expect(getByTestId('pending-signup-code-apply')).toBeDisabled();
    });

    it('activates the account with a code that holds', async () => {
      mockRedeemSignupCode.mockResolvedValueOnce({ user_status: 'active', group_joined: null });
      const view = renderComponent();

      enterCode(view, '  BETA-ABCD  ');

      await waitFor(() => expect(mockUpdateUser).toHaveBeenCalledWith({ user_status: 'active' }));
      expect(mockRedeemSignupCode).toHaveBeenCalledWith('BETA-ABCD');
      expect(view.getByText('Code accepted. Your account is ready.')).toBeTruthy();
      expect(view.queryByText("You've also joined the group this code came from.")).toBeNull();
    });

    it('asks for the confirmation link while the address is unconfirmed', async () => {
      mockUser = { email: 'jean@example.com', email_verified: false };
      mockRedeemSignupCode.mockResolvedValueOnce({ user_status: 'pending', group_joined: null });
      const view = renderComponent();

      enterCode(view, 'BETA-ABCD');

      expect(
        await view.findByText('Code accepted. Confirm your email address to activate your account.'),
      ).toBeTruthy();
      expect(mockUpdateUser).toHaveBeenCalledWith({ user_status: 'pending' });
    });

    it("says so when a coach's code also joined the group", async () => {
      mockRedeemSignupCode.mockResolvedValueOnce({ user_status: 'active', group_joined: true });
      const view = renderComponent();

      enterCode(view, 'COACH123');

      expect(await view.findByText("You've also joined the group this code came from.")).toBeTruthy();
      expect(view.getByText('Code accepted. Your account is ready.')).toBeTruthy();
    });

    it('shows a refused code under the field and leaves the account waiting', async () => {
      mockRedeemSignupCode.mockRejectedValueOnce(
        apiRefusal(400, {
          message: "This code isn't valid. Check it and try again.",
          details: { reason: 'invalid_signup_code' },
        }),
      );
      const view = renderComponent();

      enterCode(view, 'NOPE');

      expect(await view.findByText("This code isn't valid. Check it and try again.")).toBeTruthy();
      expect(mockUpdateUser).not.toHaveBeenCalled();
      expect(view.getByTestId('pending-signup-code-apply')).toBeEnabled();
    });

    it("keeps a rate limit's own sentence", async () => {
      mockRedeemSignupCode.mockRejectedValueOnce(
        apiRefusal(429, { message: 'Too many attempts. Try again later.' }),
      );
      const view = renderComponent();

      enterCode(view, 'BETA-ABCD');

      expect(await view.findByText('Too many attempts. Try again later.')).toBeTruthy();
    });

    it('disables the button while the code is checked', async () => {
      let resolve: (value: { user_status: 'active'; group_joined: null }) => void = () => {};
      mockRedeemSignupCode.mockReturnValueOnce(new Promise((r) => { resolve = r; }));
      const view = renderComponent();

      enterCode(view, 'BETA-ABCD');

      await waitFor(() => expect(view.getByTestId('pending-signup-code-apply')).toBeDisabled());
      expect(view.getByText('Checking…')).toBeTruthy();
      resolve({ user_status: 'active', group_joined: null });
      await waitFor(() => expect(mockUpdateUser).toHaveBeenCalled());
    });
  });
});
