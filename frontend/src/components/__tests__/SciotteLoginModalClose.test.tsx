// ABOUTME: Closing the sign-in window aborts the login it left running, so the server stops it too (carnet#1300)
// ABOUTME: Red if the request outlives the window, or if its abort reopens the window on an error
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

/** A login the provider never answers: it settles only when its signal aborts, as axios does. */
function hangingLogin(_params: unknown, options: { signal?: AbortSignal } = {}) {
  return new Promise((_resolve, reject) => {
    options.signal?.addEventListener('abort', () => reject(new Error('canceled')), { once: true });
  });
}

function modal(isOpen: boolean) {
  return (
    <ThemeProvider>
      <SciotteLoginModal isOpen={isOpen} onClose={vi.fn()} onConnected={vi.fn()} target="coros" consentRequired={false} />
    </ThemeProvider>
  );
}

describe('SciotteLoginModal — closing the window', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sciotteLogin.mockImplementation(hangingLogin);
  });

  it('aborts the login in flight and reopens on a fresh form, not an error', async () => {
    const { rerender } = render(modal(true));
    const user = userEvent.setup();
    await user.type(screen.getByLabelText('Email'), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));

    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(1));
    const { signal } = sciotteLogin.mock.calls[0][1] as { signal: AbortSignal };
    expect(signal.aborted).toBe(false);

    // Closed and reopened at once, so the aborted login settles on the reopened form.
    rerender(modal(false));
    expect(signal.aborted).toBe(true);
    rerender(modal(true));
    expect(await screen.findByLabelText('Email')).toHaveValue('');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText('Login Failed')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Email')).toBeInTheDocument();
  });
});
