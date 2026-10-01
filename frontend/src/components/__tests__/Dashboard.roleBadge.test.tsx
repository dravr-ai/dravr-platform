// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the sidebar's operator role badge to the catalogue label, in the catalogue's own casing
// ABOUTME: A CSS capitalize on the translated label would re-case a language that writes it otherwise

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import Dashboard from '../Dashboard';

const auth = vi.hoisted(() => ({ role: 'super_admin' }));

vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));

vi.mock('../UserManagement', () => ({
  default: () => <div data-testid="users-tab">Users surface</div>,
}));

vi.mock('../ImpersonationLogTab', () => ({
  default: () => <div data-testid="impersonation-log-tab">Impersonation log surface</div>,
}));

vi.mock('../ConnectProviderBanner', () => ({
  ConnectProviderBanner: () => null,
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'op-1', email: 'op@acme.com', display_name: 'Operator', role: auth.role },
    logout: vi.fn(),
    isAuthenticated: true,
    isLoading: false,
  }),
}));

vi.mock('../../services/api', () => ({
  // Read by the shared hook bindings at import; this spec asserts nothing they fetch.
  featureFlagsApi: {},
}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

function renderDashboard() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <Dashboard />
    </QueryClientProvider>,
  );
}

describe('Dashboard sidebar role badge', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it.each(['en', 'fr'])('prints the %s catalogue label as written, without re-casing it', async (language) => {
    await i18n.changeLanguage(language);
    try {
      await act(async () => {
        renderDashboard();
      });

      const label = i18n.t('shell.roleSuperAdmin');
      const badges = await screen.findAllByText(label);
      expect(badges.length).toBeGreaterThan(0);
      for (const badge of badges) {
        expect(badge).not.toHaveClass('capitalize');
      }
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
