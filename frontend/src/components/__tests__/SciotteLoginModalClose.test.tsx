// ABOUTME: Closing the sign-in window aborts the login it left running, so the server stops it too (carnet#1300)
// ABOUTME: Red if the request outlives the window, its abort reopens on an error, or the server is not told to cancel
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();
const sciotteCancel = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: (...args: unknown[]) => sciotteCancel(...args),
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
    sciotteCancel.mockResolvedValue({ cancelled: true });
  });

  async function logIn() {
    const user = userEvent.setup();
    await user.type(screen.getByLabelText('Email'), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));
  }

  /** The attempt id the `n`th login named. */
  function attemptOf(n: number): string {
    return (sciotteLogin.mock.calls[n][1] as { attemptId: string }).attemptId;
  }

  it('cancels the attempt the login named, and names the next opening afresh', async () => {
    const { rerender } = render(modal(true));
    await logIn();
    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(1));
    const first = attemptOf(0);
    expect(first).toMatch(/^[0-9a-f-]{36}$/);
    expect(sciotteCancel).not.toHaveBeenCalled();

    rerender(modal(false));
    expect(sciotteCancel).toHaveBeenCalledTimes(1);
    expect(sciotteCancel).toHaveBeenCalledWith(first);

    rerender(modal(true));
    await logIn();
    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(2));
    expect(attemptOf(1)).not.toBe(first);
    rerender(modal(false));
    expect(sciotteCancel.mock.calls).toEqual([[first], [attemptOf(1)]]);
  });

  it('cancels a flow parked on a code when the window closes', async () => {
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
    const { rerender } = render(modal(true));
    await logIn();
    expect(await screen.findByLabelText(/code/i)).toBeInTheDocument();

    rerender(modal(false));

    expect(sciotteCancel).toHaveBeenCalledWith(attemptOf(0));
  });

  it('sends no cancel when nothing was in flight', async () => {
    const { rerender, unmount } = render(modal(true));
    rerender(modal(false));
    unmount();

    expect(sciotteCancel).not.toHaveBeenCalled();
  });

  it('sends no cancel after a refused login', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'bad password' });
    const { rerender } = render(modal(true));
    await logIn();
    expect(await screen.findByText('Login Failed')).toBeInTheDocument();

    rerender(modal(false));

    expect(sciotteCancel).not.toHaveBeenCalled();
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
