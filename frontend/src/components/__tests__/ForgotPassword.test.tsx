// ABOUTME: Tests for the ForgotPassword component
// ABOUTME: Verifies email form, validation, API calls, and navigation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ForgotPassword from '../ForgotPassword';

vi.mock('../../services/api', () => ({
  authApi: {
    forgotPassword: vi.fn().mockResolvedValue({ message: 'Reset code sent' }),
  },
}));

import { authApi } from '../../services/api';

describe('ForgotPassword', () => {
  const mockNavigateToLogin = vi.fn();
  const mockCodeSent = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  function renderComponent() {
    return render(
      <ForgotPassword
        onNavigateToLogin={mockNavigateToLogin}
        onCodeSent={mockCodeSent}
      />,
    );
  }

  describe('rendering', () => {
    it('should render the heading and description', () => {
      renderComponent();
      expect(screen.getByText('Reset Your Password')).toBeInTheDocument();
      expect(screen.getByText(/send you a 6-digit code/i)).toBeInTheDocument();
    });

    it('should render the email input and submit button', () => {
      renderComponent();
      expect(screen.getByLabelText('Email address')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /send reset code/i })).toBeInTheDocument();
    });

    it('should render back to sign in link', () => {
      renderComponent();
      expect(screen.getByRole('button', { name: /back to sign in/i })).toBeInTheDocument();
    });
  });

  describe('form submission', () => {
    it('should call forgotPassword API on valid submit', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Email address'), 'test@example.com');
      await user.click(screen.getByRole('button', { name: /send reset code/i }));

      await waitFor(() => {
        expect(authApi.forgotPassword).toHaveBeenCalledWith('test@example.com');
      });
    });

    it('should call onCodeSent with email on success', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Email address'), 'test@example.com');
      await user.click(screen.getByRole('button', { name: /send reset code/i }));

      await waitFor(() => {
        expect(mockCodeSent).toHaveBeenCalledWith('test@example.com');
      });
    });

    it('should not call API when email is empty', async () => {
      const user = userEvent.setup();
      renderComponent();

      // The email input has the required attribute, so native validation prevents submit
      expect(screen.getByLabelText('Email address')).toBeRequired();

      await user.click(screen.getByRole('button', { name: /send reset code/i }));

      expect(authApi.forgotPassword).not.toHaveBeenCalled();
    });

    it('should display API error on failure', async () => {
      vi.mocked(authApi.forgotPassword).mockRejectedValueOnce({
        response: { data: { error: 'Server error' } },
      });

      const user = userEvent.setup();
      renderComponent();

      await user.type(screen.getByLabelText('Email address'), 'test@example.com');
      await user.click(screen.getByRole('button', { name: /send reset code/i }));

      await waitFor(() => {
        expect(screen.getByRole('alert')).toHaveTextContent('Server error');
      });
    });
  });

  describe('navigation', () => {
    it('should call onNavigateToLogin when back link is clicked', async () => {
      const user = userEvent.setup();
      renderComponent();

      await user.click(screen.getByRole('button', { name: /back to sign in/i }));

      expect(mockNavigateToLogin).toHaveBeenCalled();
    });
  });
});
