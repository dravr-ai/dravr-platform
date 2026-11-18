// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the admin's sign-in mode (carnet#1631): kept across a reload, asked again at every sign-in and after sign-out
// ABOUTME: A choice counts only for the admin who made it; an athlete never holds one

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '../AuthContext';
import { useAuth } from '../../hooks/useAuth';

const { mockAuthStorage } = vi.hoisted(() => ({
  mockAuthStorage: {
    setCsrfToken: vi.fn().mockResolvedValue(undefined),
    getCsrfToken: vi.fn().mockResolvedValue(null),
    setUser: vi.fn().mockResolvedValue(undefined),
    getUser: vi.fn().mockResolvedValue(null),
    clear: vi.fn().mockResolvedValue(undefined),
    getToken: vi.fn().mockResolvedValue(null),
    setToken: vi.fn().mockResolvedValue(undefined),
    removeToken: vi.fn().mockResolvedValue(undefined),
    getRefreshToken: vi.fn().mockResolvedValue(null),
    setRefreshToken: vi.fn().mockResolvedValue(undefined),
  },
}));

vi.mock('../../services/api', () => ({
  authApi: {
    beginSignIn: vi.fn(),
    completeSignIn: vi.fn(),
    logout: vi.fn().mockResolvedValue(undefined),
    getSession: vi.fn(),
    loginWithFirebase: vi.fn(),
  },
  adminApi: { endImpersonation: vi.fn() },
  userApi: { updateTimezone: vi.fn().mockResolvedValue(undefined) },
  pierreApi: { adapter: { authStorage: mockAuthStorage } },
}));

const VIEW_MODE_KEY = 'pierre_view_mode';
const ADMIN = { id: 'admin-1', email: 'jf@dravr.ai', role: 'admin', is_admin: true, user_status: 'active' };
const ATHLETE = { id: 'user-1', email: 'alice@acme.com', role: 'user', is_admin: false, user_status: 'active' };

function Probe() {
  const { user, viewMode, chooseViewMode, logout, loginWithFirebase } = useAuth();
  return (
    <div>
      <div data-testid="who">{user?.email ?? 'nobody'}</div>
      <div data-testid="mode">{viewMode ?? 'none'}</div>
      <button onClick={() => chooseViewMode('athlete')}>athlete</button>
      <button onClick={() => chooseViewMode('admin')}>admin</button>
      <button onClick={logout}>logout</button>
      <button onClick={() => void loginWithFirebase('id-token')}>google</button>
    </div>
  );
}

function renderProbe() {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <AuthProvider>
        <Probe />
      </AuthProvider>
    </QueryClientProvider>,
  );
}

async function restoreAs(user: typeof ADMIN | typeof ATHLETE) {
  const { authApi } = await import('../../services/api');
  localStorage.setItem('pierre_user', JSON.stringify(user));
  vi.mocked(authApi.getSession).mockResolvedValue({
    access_token: 'jwt',
    csrf_token: 'csrf',
    user,
  } as never);
}

describe('AuthContext view mode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it('starts with no mode for an admin, records the choice, and keeps it across a reload', async () => {
    await restoreAs(ADMIN);
    const first = renderProbe();
    await waitFor(() => expect(screen.getByTestId('who')).toHaveTextContent('jf@dravr.ai'));
    expect(screen.getByTestId('mode')).toHaveTextContent('none');

    await userEvent.click(screen.getByText('athlete'));
    expect(screen.getByTestId('mode')).toHaveTextContent('athlete');
    expect(JSON.parse(localStorage.getItem(VIEW_MODE_KEY) ?? 'null')).toEqual({ userId: 'admin-1', mode: 'athlete' });

    // A reload: a fresh provider over the same browser storage.
    first.unmount();
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('who')).toHaveTextContent('jf@dravr.ai'));
    expect(screen.getByTestId('mode')).toHaveTextContent('athlete');
  });

  it('forgets the mode at sign-out', async () => {
    await restoreAs(ADMIN);
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('who')).toHaveTextContent('jf@dravr.ai'));
    await userEvent.click(screen.getByText('admin'));
    expect(screen.getByTestId('mode')).toHaveTextContent('admin');

    await userEvent.click(screen.getByText('logout'));
    await waitFor(() => expect(screen.getByTestId('who')).toHaveTextContent('nobody'));
    expect(screen.getByTestId('mode')).toHaveTextContent('none');
    expect(localStorage.getItem(VIEW_MODE_KEY)).toBeNull();
  });

  it('asks again at the next sign-in, even for the same admin', async () => {
    await restoreAs(ADMIN);
    localStorage.setItem(VIEW_MODE_KEY, JSON.stringify({ userId: 'admin-1', mode: 'athlete' }));
    const { authApi } = await import('../../services/api');
    vi.mocked(authApi.loginWithFirebase).mockResolvedValue({
      csrf_token: 'csrf',
      jwt_token: 'jwt',
      user: ADMIN,
    } as never);
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('mode')).toHaveTextContent('athlete'));

    await act(async () => {
      await userEvent.click(screen.getByText('google'));
    });
    await waitFor(() => expect(screen.getByTestId('mode')).toHaveTextContent('none'));
    expect(localStorage.getItem(VIEW_MODE_KEY)).toBeNull();
  });

  it('ignores a choice another admin made on this browser', async () => {
    await restoreAs(ADMIN);
    localStorage.setItem(VIEW_MODE_KEY, JSON.stringify({ userId: 'someone-else', mode: 'athlete' }));
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('who')).toHaveTextContent('jf@dravr.ai'));
    expect(screen.getByTestId('mode')).toHaveTextContent('none');
  });

  it('never gives an athlete a mode, whatever the browser kept', async () => {
    await restoreAs(ATHLETE);
    localStorage.setItem(VIEW_MODE_KEY, JSON.stringify({ userId: 'user-1', mode: 'admin' }));
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('who')).toHaveTextContent('alice@acme.com'));
    expect(screen.getByTestId('mode')).toHaveTextContent('none');

    await userEvent.click(screen.getByText('admin'));
    expect(screen.getByTestId('mode')).toHaveTextContent('none');
  });

  it('asks an admin whose session names only user_id, and keeps the choice for that id', async () => {
    const onlyUserId = {
      user_id: 'admin-1',
      email: ADMIN.email,
      role: ADMIN.role,
      is_admin: ADMIN.is_admin,
      user_status: ADMIN.user_status,
    };
    await restoreAs(onlyUserId as unknown as typeof ADMIN);
    renderProbe();
    await waitFor(() => expect(screen.getByTestId('who')).toHaveTextContent('jf@dravr.ai'));
    expect(screen.getByTestId('mode')).toHaveTextContent('none');

    await userEvent.click(screen.getByText('athlete'));
    expect(screen.getByTestId('mode')).toHaveTextContent('athlete');
    expect(JSON.parse(localStorage.getItem(VIEW_MODE_KEY) ?? 'null')).toEqual({ userId: 'admin-1', mode: 'athlete' });
  });
});
