// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web self-serve account deletion: typed-email gate, password, blockers, sign-out after delete
// ABOUTME: The delete button stays disabled until the confirmation matches; a 409 lists each blocker in the app language

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import AccountDeletionSection from '../AccountDeletionSection';

const getAccountDeletionPreview = vi.fn();
const deleteAccount = vi.fn();
const logout = vi.fn();
const deleteFirebaseAccount = vi.fn();

vi.mock('../../services/api', () => ({
  userApi: {
    getAccountDeletionPreview: () => getAccountDeletionPreview(),
    deleteAccount: (request: unknown) => deleteAccount(request),
  },
}));
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ logout }) }));
vi.mock('../../firebase/firebase', () => ({
  deleteFirebaseAccount: () => deleteFirebaseAccount(),
}));

function renderSection() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AccountDeletionSection />
    </QueryClientProvider>,
  );
}

describe('AccountDeletionSection', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    await i18n.changeLanguage('en');
  });

  it('deletes only once the email matches and the password is given, then signs out', async () => {
    getAccountDeletionPreview.mockResolvedValue({
      email: 'athlete@example.com',
      requires_password: true,
      providers: ['strava'],
      blockers: [],
    });
    deleteAccount.mockResolvedValue({ message: 'ok', disconnected_providers: ['strava'] });
    deleteFirebaseAccount.mockResolvedValue(false);
    renderSection();

    fireEvent.click(screen.getByTestId('account-deletion-open'));
    const email = await screen.findByTestId('account-deletion-email');
    expect(screen.getByTestId('account-deletion-providers')).toHaveTextContent(
      'These providers will be disconnected: strava',
    );
    const confirm = screen.getByTestId('account-deletion-confirm');
    expect(confirm).toBeDisabled();

    fireEvent.change(email, { target: { value: 'Athlete@Example.com' } });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByTestId('account-deletion-password'), {
      target: { value: 'secret' },
    });
    expect(confirm).toBeEnabled();

    fireEvent.click(confirm);
    await waitFor(() => expect(logout).toHaveBeenCalledTimes(1));
    expect(deleteAccount).toHaveBeenCalledWith({
      confirm_email: 'Athlete@Example.com',
      password: 'secret',
    });
    expect(deleteFirebaseAccount).toHaveBeenCalledTimes(1);
  });

  it('signs out after the delete even when Firebase fails to delete the identity', async () => {
    getAccountDeletionPreview.mockResolvedValue({
      email: 'athlete@example.com',
      requires_password: false,
      providers: [],
      blockers: [],
    });
    deleteAccount.mockResolvedValue({ message: 'ok', disconnected_providers: [] });
    deleteFirebaseAccount.mockRejectedValue(new Error('auth/network-request-failed'));
    renderSection();

    fireEvent.click(screen.getByTestId('account-deletion-open'));
    fireEvent.change(await screen.findByTestId('account-deletion-email'), {
      target: { value: 'athlete@example.com' },
    });
    fireEvent.click(screen.getByTestId('account-deletion-confirm'));

    await waitFor(() => expect(logout).toHaveBeenCalledTimes(1));
    expect(deleteFirebaseAccount).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('account-deletion-error')).toBeNull();
  });

  it('lists what blocks the delete and offers no delete button', async () => {
    getAccountDeletionPreview.mockResolvedValue({
      email: 'coach@example.com',
      requires_password: false,
      providers: [],
      blockers: [{ kind: 'owns_coaching_group', detail: 'Track Tuesdays' }],
    });
    renderSection();

    fireEvent.click(screen.getByTestId('account-deletion-open'));
    expect(await screen.findByTestId('account-deletion-blockers')).toHaveTextContent(
      'You own the group “Track Tuesdays”',
    );
    expect(screen.queryByTestId('account-deletion-confirm')).toBeNull();
    expect(deleteAccount).not.toHaveBeenCalled();
  });

  it('says the password is wrong in French and keeps the athlete signed in', async () => {
    await i18n.changeLanguage('fr');
    getAccountDeletionPreview.mockResolvedValue({
      email: 'athlete@example.com',
      requires_password: true,
      providers: [],
      blockers: [],
    });
    deleteAccount.mockRejectedValue({
      response: { status: 403, data: { error: 'password_incorrect', message: 'The password is incorrect' } },
    });
    renderSection();

    fireEvent.click(screen.getByTestId('account-deletion-open'));
    fireEvent.change(await screen.findByTestId('account-deletion-email'), {
      target: { value: 'athlete@example.com' },
    });
    fireEvent.change(screen.getByTestId('account-deletion-password'), {
      target: { value: 'wrong' },
    });
    fireEvent.click(screen.getByTestId('account-deletion-confirm'));

    expect(await screen.findByTestId('account-deletion-error')).toHaveTextContent(
      'Ce mot de passe est incorrect.',
    );
    expect(logout).not.toHaveBeenCalled();
  });
});
