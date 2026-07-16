// ABOUTME: Tests for the UserDetailDrawer tier override control (super-admin only)
// ABOUTME: Mocks adminApi + useAuth; asserts setUserTier/clearUserTier args and role gating
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import UserDetailDrawer from '../UserDetailDrawer';
import type { User } from '../../types/api';

const getUserRateLimit = vi.fn();
const getUserActivity = vi.fn();
const getUserUsage = vi.fn();
const getUserAdminProfile = vi.fn();
const setUserTier = vi.fn();
const clearUserTier = vi.fn();

// Mock API service - factory must be self-contained (vi.mock is hoisted)
vi.mock('../../services/api', () => ({
  adminApi: {
    getUserRateLimit: (...args: unknown[]) => getUserRateLimit(...args),
    getUserActivity: (...args: unknown[]) => getUserActivity(...args),
    getUserUsage: (...args: unknown[]) => getUserUsage(...args),
    getUserAdminProfile: (...args: unknown[]) => getUserAdminProfile(...args),
    setUserTier: (...args: unknown[]) => setUserTier(...args),
    clearUserTier: (...args: unknown[]) => clearUserTier(...args),
    setUserRateLimitOverride: vi.fn(),
    clearUserRateLimitOverride: vi.fn(),
  },
}));

// The feature-flags panel has its own tests and its own API surface
vi.mock('../FeatureFlagsPanel', () => ({
  default: () => null,
}));

vi.mock('../PasswordResetModal', () => ({
  default: () => null,
}));

// Role is mutable so tests can flip between super_admin and admin
let mockRole: 'super_admin' | 'admin' = 'super_admin';
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'admin-1',
      email: 'admin@example.com',
      is_admin: true,
      role: mockRole,
      tier: 'enterprise',
      created_at: '2026-01-01T00:00:00Z',
    },
    startImpersonation: vi.fn(),
  }),
}));

const targetUser: User = {
  id: 'user-42',
  email: 'athlete@example.com',
  display_name: 'Athlete One',
  is_admin: false,
  role: 'user',
  user_status: 'active',
  tier: 'starter',
  tenant_id: 'tenant-a',
  created_at: '2026-01-01T00:00:00Z',
};

function renderDrawer(user: User = targetUser) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <UserDetailDrawer
        user={user}
        isOpen={true}
        onClose={vi.fn()}
        onAction={vi.fn()}
      />
    </QueryClientProvider>,
  );
}

async function renderAndSettle(user: User = targetUser) {
  renderDrawer(user);
  // Wait for the rate-limit query so state updates stay inside the test body
  await waitFor(() => {
    expect(screen.getByText('Daily Usage')).toBeInTheDocument();
  });
}

describe('UserDetailDrawer tier control', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRole = 'super_admin';
    getUserRateLimit.mockResolvedValue({
      user_id: 'user-42',
      tier: 'starter',
      rate_limits: {
        daily: { limit: 100, used: 5, remaining: 95 },
        monthly: { limit: 3000, used: 50, remaining: 2950 },
      },
      reset_times: {
        daily_reset: '2026-07-17T00:00:00Z',
        monthly_reset: '2026-08-01T00:00:00Z',
      },
      override_active: false,
      override_note: null,
    });
    getUserActivity.mockResolvedValue({
      user_id: 'user-42',
      period_days: 30,
      total_requests: 0,
      top_tools: [],
    });
    getUserUsage.mockResolvedValue({
      user_id: 'user-42',
      from: '2026-07-01T00:00:00Z',
      by_model: [],
      total_cost_usd: 0,
      daily: [],
    });
    getUserAdminProfile.mockResolvedValue({
      user_id: 'user-42',
      coaching_persona: 'supportive_coach',
      default_coach_id: null,
      installed_coaches: [],
      joined_groups: [],
    });
    setUserTier.mockResolvedValue({
      user_id: 'user-42',
      email: 'athlete@example.com',
      tier: 'professional',
    });
    clearUserTier.mockResolvedValue({ removed: true });
  });

  it('saves the selected tier via setUserTier for super-admins', async () => {
    await renderAndSettle();

    fireEvent.click(screen.getByText('Edit tier'));

    fireEvent.change(screen.getByLabelText('Tier'), {
      target: { value: 'professional' },
    });
    fireEvent.click(screen.getByText('Save tier'));

    await waitFor(() => {
      expect(setUserTier).toHaveBeenCalledWith('user-42', 'professional');
    });
  });

  it('clears the tier override via clearUserTier', async () => {
    await renderAndSettle();

    fireEvent.click(screen.getByText('Edit tier'));
    fireEvent.click(screen.getByText('Clear override'));

    await waitFor(() => {
      expect(clearUserTier).toHaveBeenCalledWith('user-42');
    });
    expect(setUserTier).not.toHaveBeenCalled();
  });

  it('preselects the current tier even when the API serves the display form', async () => {
    // The admin users list serves "Professional" (capitalized), while the
    // select values are lowercase. Regression: the editor used to fall back
    // to Starter, so a blind Save silently downgraded the user.
    await renderAndSettle({ ...targetUser, tier: 'Professional' });

    fireEvent.click(screen.getByText('Edit tier'));

    expect((screen.getByLabelText('Tier') as HTMLSelectElement).value).toBe(
      'professional',
    );
    fireEvent.click(screen.getByText('Save tier'));
    await waitFor(() => {
      expect(setUserTier).toHaveBeenCalledWith('user-42', 'professional');
    });
  });

  it('hides the tier editor for non-super-admin admins', async () => {
    mockRole = 'admin';
    await renderAndSettle();

    expect(screen.queryByText('Edit tier')).not.toBeInTheDocument();
    // Static tier badge still shows
    expect(screen.getByText('starter')).toBeInTheDocument();
  });
});
