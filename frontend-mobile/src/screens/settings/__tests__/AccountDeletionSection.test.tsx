// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile self-serve account deletion sheet: typed-email gate, password, blockers, sign-out
// ABOUTME: The delete stays disabled until the confirmation matches; a refused delete keeps the athlete signed in

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AccountDeletionSection } from '../AccountDeletionSection';
import { userApi } from '../../../services/api';

const mockLogout = jest.fn();

jest.mock('../../../services/api', () => ({
  userApi: {
    getAccountDeletionPreview: jest.fn(),
    deleteAccount: jest.fn(),
  },
}));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ logout: mockLogout }),
}));

const getPreview = userApi.getAccountDeletionPreview as jest.Mock;
const deleteAccount = userApi.deleteAccount as jest.Mock;

function renderSection() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <AccountDeletionSection />
    </QueryClientProvider>,
  );
}

function isDisabled(testID: string): boolean {
  return Boolean(screen.getByTestId(testID).props.accessibilityState?.disabled);
}

describe('AccountDeletionSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockLogout.mockResolvedValue(undefined);
  });

  it('deletes once the email matches and the password is given, then signs out', async () => {
    getPreview.mockResolvedValue({
      email: 'athlete@example.com',
      requires_password: true,
      providers: ['strava'],
      blockers: [],
    });
    deleteAccount.mockResolvedValue({ message: 'ok', disconnected_providers: ['strava'] });
    renderSection();

    fireEvent.press(screen.getByTestId('account-deletion-open'));
    const email = await screen.findByTestId('account-deletion-email');
    expect(isDisabled('account-deletion-confirm')).toBe(true);

    fireEvent.changeText(email, ' ATHLETE@example.com');
    expect(isDisabled('account-deletion-confirm')).toBe(true);
    fireEvent.changeText(screen.getByTestId('account-deletion-password'), 'secret');
    expect(isDisabled('account-deletion-confirm')).toBe(false);

    fireEvent.press(screen.getByTestId('account-deletion-confirm'));
    await waitFor(() => expect(mockLogout).toHaveBeenCalledTimes(1));
    expect(deleteAccount).toHaveBeenCalledWith({
      confirm_email: ' ATHLETE@example.com',
      password: 'secret',
    });
  });

  it('deletes a federated account with its email alone, then signs out', async () => {
    getPreview.mockResolvedValue({
      email: 'athlete@example.com',
      requires_password: false,
      providers: [],
      blockers: [],
    });
    deleteAccount.mockResolvedValue({ message: 'ok', disconnected_providers: [] });
    renderSection();

    fireEvent.press(screen.getByTestId('account-deletion-open'));
    fireEvent.changeText(await screen.findByTestId('account-deletion-email'), 'athlete@example.com');
    fireEvent.press(screen.getByTestId('account-deletion-confirm'));

    await waitFor(() => expect(mockLogout).toHaveBeenCalledTimes(1));
    expect(deleteAccount).toHaveBeenCalledWith({
      confirm_email: 'athlete@example.com',
      password: undefined,
    });
    expect(screen.queryByTestId('account-deletion-error')).toBeNull();
  });

  it('lists what blocks the delete and offers no delete button', async () => {
    getPreview.mockResolvedValue({
      email: 'coach@example.com',
      requires_password: false,
      providers: [],
      blockers: [{ kind: 'billing_subscription', detail: 'pro plan with stripe (active)' }],
    });
    renderSection();

    fireEvent.press(screen.getByTestId('account-deletion-open'));
    expect(await screen.findByTestId('account-deletion-blockers')).toBeTruthy();
    expect(screen.getByText(/pro plan with stripe \(active\)/)).toBeTruthy();
    expect(screen.queryByTestId('account-deletion-confirm')).toBeNull();
  });

  it('shows the refusal and keeps the athlete signed in when the password is wrong', async () => {
    getPreview.mockResolvedValue({
      email: 'athlete@example.com',
      requires_password: true,
      providers: [],
      blockers: [],
    });
    deleteAccount.mockRejectedValue({
      response: { status: 403, data: { error: 'password_incorrect' } },
    });
    renderSection();

    fireEvent.press(screen.getByTestId('account-deletion-open'));
    fireEvent.changeText(await screen.findByTestId('account-deletion-email'), 'athlete@example.com');
    fireEvent.changeText(screen.getByTestId('account-deletion-password'), 'wrong');
    fireEvent.press(screen.getByTestId('account-deletion-confirm'));

    expect(await screen.findByTestId('account-deletion-error')).toBeTruthy();
    expect(mockLogout).not.toHaveBeenCalled();
  });
});
