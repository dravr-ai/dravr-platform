// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins who the users list offers to suspend: every active or pending account, another admin included, never the viewer's own
// ABOUTME: The server refuses the last active admin, and the suspend dialog shows the sentence it answers with

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import UserManagement from '../UserManagement';
import { LAST_ADMIN_REFUSAL, SUSPEND_OFFER_CASES } from './suspendOfferCases';

const getAllUsers = vi.fn();
const suspendUser = vi.fn();

// The signed-in admin whose own row is asked about.
vi.mock('../../hooks/useAuth', async () => {
  const { VIEWER } = await import('./suspendOfferCases');
  return { useAuth: () => ({ user: VIEWER }) };
});

vi.mock('../../services/api', () => ({
  adminApi: {
    getPendingUsers: () => Promise.resolve([]),
    getAllUsers: (...args: unknown[]) => getAllUsers(...args),
    suspendUser: (...args: unknown[]) => suspendUser(...args),
  },
  coachAccessAdminApi: { list: () => Promise.resolve([]) },
}));

// Each of these owns its own queries and has its own spec.
vi.mock('../PendingUsersList', () => ({ default: () => null }));
vi.mock('../PreApprovedEmails', () => ({ default: () => null }));
vi.mock('../SignupCodes', () => ({ default: () => null }));
vi.mock('../CoachAccessRequests', () => ({ default: () => null }));
vi.mock('../NewUsersView', () => ({ default: () => null }));
vi.mock('../UserDetailDrawer', () => ({ default: () => null }));

async function rowOf(email: string): Promise<HTMLElement> {
  const cell = await screen.findByText(email, { selector: 'p' });
  const line = cell.closest('div.cursor-pointer');
  expect(line, `${email} row`).not.toBeNull();
  return line as HTMLElement;
}

/** The list on its All Users tab, where every status has a row. */
async function renderAllUsers() {
  getAllUsers.mockResolvedValue(SUSPEND_OFFER_CASES.map(({ user }) => user));
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <UserManagement />
    </QueryClientProvider>,
  );
  fireEvent.click(await screen.findByRole('button', { name: /^All Users/ }));
}

describe('UserManagement suspend offer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it.each(SUSPEND_OFFER_CASES)('on $name: offered is $offered', async ({ user, offered }) => {
    await renderAllUsers();
    const row = await rowOf(user.email);
    expect(within(row).queryByRole('button', { name: 'Suspend' }) !== null).toBe(offered);
  });

  it('suspends another admin, sending the request the server takes', async () => {
    suspendUser.mockResolvedValue({ success: true, message: 'User suspended successfully' });
    await renderAllUsers();
    fireEvent.click(within(await rowOf('admin-2@example.com')).getByRole('button', { name: 'Suspend' }));

    const dialog = await screen.findByRole('dialog', { name: 'Suspend User' });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Suspend User' }));

    await vi.waitFor(() => expect(suspendUser).toHaveBeenCalledWith('admin-2', undefined));
    await vi.waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('suspends a pending account from its row, beside Approve', async () => {
    suspendUser.mockResolvedValue({ success: true, message: 'User suspended successfully' });
    await renderAllUsers();
    const row = await rowOf('pending-user@example.com');
    expect(within(row).getByRole('button', { name: 'Approve' })).toBeInTheDocument();
    fireEvent.click(within(row).getByRole('button', { name: 'Suspend' }));

    const dialog = await screen.findByRole('dialog', { name: 'Suspend User' });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Suspend User' }));

    await vi.waitFor(() => expect(suspendUser).toHaveBeenCalledWith('pending-user', undefined));
    await vi.waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it("shows the server's sentence when it refuses the last active admin, and not on the next account", async () => {
    suspendUser.mockRejectedValue(LAST_ADMIN_REFUSAL);
    await renderAllUsers();
    fireEvent.click(within(await rowOf('admin-2@example.com')).getByRole('button', { name: 'Suspend' }));

    const dialog = await screen.findByRole('dialog', { name: 'Suspend User' });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Suspend User' }));
    expect(await within(dialog).findByRole('alert')).toHaveTextContent(
      /^Cannot suspend the last active admin$/,
    );
    expect(suspendUser).toHaveBeenCalledWith('admin-2', undefined);

    // The dialog stays mounted between accounts: the refusal belongs to the one it was about.
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    fireEvent.click(within(await rowOf('athlete@example.com')).getByRole('button', { name: 'Suspend' }));
    const reopened = await screen.findByRole('dialog', { name: 'Suspend User' });
    expect(within(reopened).queryByRole('alert')).toBeNull();
  });
});
