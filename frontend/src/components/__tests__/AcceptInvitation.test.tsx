// ABOUTME: Tests for the invitation landing page — reading the token, choosing a password, and every outcome
// ABOUTME: Asserts what the invitee sees and which call goes out, not merely that the page rendered
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import AcceptInvitation from '../AcceptInvitation';

vi.mock('../../services/api', () => ({
  invitationsApi: {
    acceptInvitation: vi.fn(),
  },
}));

import { invitationsApi } from '../../services/api';

const TOKEN = 'AbCd1234EfGh5678.IjKl9012MnOp3456QrSt7890UvWx1234';

describe('AcceptInvitation', () => {
  const onSignIn = vi.fn();
  const onRegister = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  function renderPage(token: string | null = TOKEN) {
    return render(<AcceptInvitation token={token} onSignIn={onSignIn} onRegister={onRegister} />);
  }

  async function fillAndSubmit(password: string, confirm: string, name?: string) {
    const user = userEvent.setup();
    if (name) {
      await user.type(screen.getByLabelText('Display Name'), name);
    }
    await user.type(screen.getByLabelText('Password'), password);
    await user.type(screen.getByLabelText('Confirm Password'), confirm);
    await user.click(screen.getByRole('button', { name: 'Create Account' }));
  }

  it('asks for a name and a password, never an address', () => {
    renderPage();
    expect(screen.getByRole('heading', { name: 'Create Your Account' })).toBeTruthy();
    expect(screen.getByLabelText('Display Name')).toBeTruthy();
    expect(screen.getByLabelText('Password')).toBeTruthy();
    expect(screen.getByLabelText('Confirm Password')).toBeTruthy();
    expect(screen.queryByLabelText('Email address')).toBeNull();
  });

  it('redeems the token with the chosen password and name, then offers sign-in', async () => {
    vi.mocked(invitationsApi.acceptInvitation).mockResolvedValue({
      user_id: 'user-1',
      email: 'coach@example.com',
      user_status: 'active',
      display_name: 'Coach Phil',
      message: 'Your account is ready.',
    });
    renderPage();

    await fillAndSubmit('StrongPass1', 'StrongPass1', 'Coach Phil');

    await waitFor(() =>
      expect(invitationsApi.acceptInvitation).toHaveBeenCalledWith(
        TOKEN,
        'StrongPass1',
        'Coach Phil',
        false,
      ),
    );
    expect(await screen.findByRole('heading', { name: 'Account created' })).toBeTruthy();
    // The invitee never typed the address: it is named before they sign in with it.
    expect(screen.getByTestId('accepted-email')).toHaveTextContent('coach@example.com');
    expect(screen.getByText('Then connect a fitness service and meet your agent')).toBeTruthy();

    await userEvent.setup().click(screen.getByRole('button', { name: 'Continue to sign in' }));
    expect(onSignIn).toHaveBeenCalledTimes(1);
    expect(onRegister).not.toHaveBeenCalled();
  });

  it('sends no name when the field is left empty', async () => {
    vi.mocked(invitationsApi.acceptInvitation).mockResolvedValue({
      user_id: 'user-2',
      email: 'athlete@example.com',
      user_status: 'active',
      message: 'Your account is ready.',
    });
    renderPage();

    await fillAndSubmit('StrongPass1', 'StrongPass1');

    await waitFor(() =>
      expect(invitationsApi.acceptInvitation).toHaveBeenCalledWith(
        TOKEN,
        'StrongPass1',
        undefined,
        false,
      ),
    );
  });

  it('asks for usage-analytics consent with its own unticked box, and sends a yes once ticked', async () => {
    vi.mocked(invitationsApi.acceptInvitation).mockResolvedValue({
      user_id: 'user-3',
      email: 'athlete@example.com',
      user_status: 'active',
      message: 'Your account is ready.',
    });
    renderPage();

    const box = screen.getByRole('checkbox', { name: /Help improve Dravr with usage analytics/ });
    expect(box).not.toBeChecked();
    await userEvent.setup().click(box);
    await fillAndSubmit('StrongPass1', 'StrongPass1');

    await waitFor(() =>
      expect(invitationsApi.acceptInvitation).toHaveBeenCalledWith(
        TOKEN,
        'StrongPass1',
        undefined,
        true,
      ),
    );
  });

  it('refuses a short password before spending the invitation', async () => {
    renderPage();

    await fillAndSubmit('short', 'short');

    expect(screen.getByRole('alert').textContent).toBe('Password must be at least 8 characters');
    expect(invitationsApi.acceptInvitation).not.toHaveBeenCalled();
  });

  it('refuses mismatched passwords before spending the invitation', async () => {
    renderPage();

    await fillAndSubmit('StrongPass1', 'StrongPass2');

    expect(screen.getByRole('alert').textContent).toBe('Passwords do not match');
    expect(invitationsApi.acceptInvitation).not.toHaveBeenCalled();
  });

  it('turns a spent or expired link into a way to sign up or sign in', async () => {
    vi.mocked(invitationsApi.acceptInvitation).mockRejectedValue({
      response: { status: 404, data: { message: 'Invitation link is invalid, expired, or already used' } },
    });
    renderPage();

    await fillAndSubmit('StrongPass1', 'StrongPass1');

    expect(await screen.findByRole('heading', { name: 'That link has expired' })).toBeTruthy();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Create an account' }));
    expect(onRegister).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Back to sign in' }));
    expect(onSignIn).toHaveBeenCalledTimes(1);
  });

  it('shows the expired state at once for a link that carried no token', () => {
    renderPage(null);

    expect(screen.getByRole('heading', { name: 'That link has expired' })).toBeTruthy();
    expect(screen.queryByLabelText('Password')).toBeNull();
    expect(invitationsApi.acceptInvitation).not.toHaveBeenCalled();
  });

  it('keeps the form and shows the server refusal for any other failure', async () => {
    vi.mocked(invitationsApi.acceptInvitation).mockRejectedValue({
      response: { status: 403, data: { message: 'User state error: User already exists' } },
    });
    renderPage();

    await fillAndSubmit('StrongPass1', 'StrongPass1');

    expect((await screen.findByRole('alert')).textContent).toBe('User state error: User already exists');
    expect(screen.getByRole('heading', { name: 'Create Your Account' })).toBeTruthy();
  });

  describe('a team-issued coach-seat link', () => {
    function renderCoachPage(token: string | null = TOKEN) {
      return render(
        <AcceptInvitation token={token} kind="coach_seat" onSignIn={onSignIn} onRegister={onRegister} />,
      );
    }

    it('says the coach seat comes with the account and the address is the invitation one', () => {
      renderCoachPage();
      expect(screen.getByRole('heading', { name: 'Create your coach account' })).toBeTruthy(); // humanCoach.seatInvite.title
      expect(screen.getByText('An invitation from the Dravr team: your coach seat is included.')).toBeTruthy(); // humanCoach.seatInvite.subtitle
      expect(screen.getByTestId('invite-email-locked').textContent).toBe(
        'Your account is created with the address your invitation was sent to, and no other.',
      );
      expect(screen.queryByLabelText('Email address')).toBeNull();
    });

    it('names the address and says the seat is already active once the account exists', async () => {
      vi.mocked(invitationsApi.acceptInvitation).mockResolvedValue({
        user_id: 'user-1',
        email: 'coach@example.com',
        user_status: 'active',
        display_name: null,
        message: 'Your account is ready.',
      });
      renderCoachPage();

      await fillAndSubmit('StrongPass1', 'StrongPass1');

      expect(await screen.findByRole('heading', { name: 'Your coach account is ready' })).toBeTruthy();
      expect(screen.getByTestId('accepted-email').textContent).toContain('coach@example.com');
      expect(
        screen.getByText('Sign in with this address and your password: your coach seat is already active.'), // humanCoach.seatInvite.doneBody
      ).toBeTruthy();
      await userEvent.setup().click(screen.getByRole('button', { name: 'Continue to sign in' }));
      expect(onSignIn).toHaveBeenCalledTimes(1);
    });

    it('refuses an expired or revoked link plainly and sends the coach back to the team', async () => {
      vi.mocked(invitationsApi.acceptInvitation).mockRejectedValue({
        response: { status: 404, data: { message: 'The requested resource was not found' } },
      });
      renderCoachPage();

      await fillAndSubmit('StrongPass1', 'StrongPass1');

      expect(await screen.findByRole('heading', { name: 'This invitation link no longer works' })).toBeTruthy();
      expect(screen.getByTestId('invite-expired-body').textContent).toContain('Ask the Dravr team to send you a new one');
      // A plain sign-up would carry no seat, so it is not the way offered.
      expect(screen.queryByRole('button', { name: 'Create an account' })).toBeNull();
      await userEvent.setup().click(screen.getByRole('button', { name: 'Back to sign in' }));
      expect(onSignIn).toHaveBeenCalledTimes(1);
    });
  });
});
