// ABOUTME: Tests for the ResetPassword component
// ABOUTME: Verifies code entry, password validation, API calls, and navigation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ResetPassword from '../ResetPassword';

vi.mock('../../services/api', () => ({
  authApi: {
    resetPassword: vi.fn().mockResolvedValue({ message: 'Password reset successfully' }),
  },
}));

import { authApi } from '../../services/api';

describe('ResetPassword', () => {
  const mockNavigateToLogin = vi.fn();
  const mockResetSuccess = vi.fn();
  const mockResendCode = vi.fn();
  const testEmail = 'test@example.com';

  beforeEach(() => {
    vi.clearAllMocks();
  });

  function renderComponent() {
    return render(
      <ResetPassword
        email={testEmail}
        onNavigateToLogin={mockNavigateToLogin}
        onResetSuccess={mockResetSuccess}
        onResendCode={mockResendCode}
      />,
    );
  }

  describe('rendering', () => {
    it('should render the heading and email', () => {
      renderComponent();
      expect(screen.getByText('Enter Reset Code')).toBeInTheDocument();
      expect(screen.getByText(testEmail)).toBeInTheDocument();
    });

    it('should render code, password, and confirm password inputs', () => {
      renderComponent();
      expect(screen.getByLabelText('Reset code')).toBeInTheDocument();
      expect(screen.getByLabelText('New password')).toBeInTheDocument();
      expect(screen.getByLabelText('Confirm new password')).toBeInTheDocument();
    });

    it('should render resend code and back links', () => {
      renderComponent();
      expect(screen.getByRole('button', { name: /resend code/i })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /back to sign in/i })).toBeInTheDocument();
    });
  });

  describe('form validation', () => {
    it('should reject non-numeric code', async () => {
      const user = userEvent.setup();
      renderComponent();

      const codeInput = screen.getByLabelText('Reset code');
      await user.type(codeInput, 'abcdef');

      // Code input should strip non-digit chars
      expect(codeInput).toHaveValue('');
    });

    it('should reject short password', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Reset code'), '123456');
      await user.type(screen.getByLabelText('New password'), 'short');
      await user.type(screen.getByLabelText('Confirm new password'), 'short');
      await user.click(screen.getByRole('button', { name: /reset password/i }));

      expect(screen.getByText(/at least 8 characters/i)).toBeInTheDocument();
      expect(authApi.resetPassword).not.toHaveBeenCalled();
    });

    it('should reject mismatched passwords', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Reset code'), '123456');
      await user.type(screen.getByLabelText('New password'), 'ValidPassword123');
      await user.type(screen.getByLabelText('Confirm new password'), 'DifferentPassword456');
      await user.click(screen.getByRole('button', { name: /reset password/i }));

      expect(screen.getByText(/do not match/i)).toBeInTheDocument();
      expect(authApi.resetPassword).not.toHaveBeenCalled();
    });
  });

  describe('form submission', () => {
    it('should call resetPassword API on valid submit', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Reset code'), '123456');
      await user.type(screen.getByLabelText('New password'), 'NewPassword123');
      await user.type(screen.getByLabelText('Confirm new password'), 'NewPassword123');
      await user.click(screen.getByRole('button', { name: /reset password/i }));

      await waitFor(() => {
        expect(authApi.resetPassword).toHaveBeenCalledWith('123456', 'NewPassword123');
      });
    });

    it('should call onResetSuccess on success', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Reset code'), '123456');
      await user.type(screen.getByLabelText('New password'), 'NewPassword123');
      await user.type(screen.getByLabelText('Confirm new password'), 'NewPassword123');
      await user.click(screen.getByRole('button', { name: /reset password/i }));

      await waitFor(() => {
        expect(mockResetSuccess).toHaveBeenCalledWith('Password reset successfully');
      });
    });

    it('should show expired code error on 404', async () => {
      vi.mocked(authApi.resetPassword).mockRejectedValueOnce({
        response: { status: 404, data: { error: 'not found' } },
      });

      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Reset code'), '123456');
      await user.type(screen.getByLabelText('New password'), 'NewPassword123');
      await user.type(screen.getByLabelText('Confirm new password'), 'NewPassword123');
      await user.click(screen.getByRole('button', { name: /reset password/i }));

      await waitFor(() => {
        expect(screen.getByRole('alert')).toHaveTextContent(/invalid or expired/i);
      });
    });
  });

  describe('navigation', () => {
    it('should call onResendCode when resend link is clicked', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.click(screen.getByRole('button', { name: /resend code/i }));

      expect(mockResendCode).toHaveBeenCalled();
    });

    it('should call onNavigateToLogin when back link is clicked', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.click(screen.getByRole('button', { name: /back to sign in/i }));

      expect(mockNavigateToLogin).toHaveBeenCalled();
    });
  });
});
