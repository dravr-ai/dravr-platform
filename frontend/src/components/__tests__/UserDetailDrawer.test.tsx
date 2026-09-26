// ABOUTME: Tests for the UserDetailDrawer tier override control (super-admin only) and its monthly rate-limit override
// ABOUTME: Mocks adminApi + useAuth; asserts setUserTier/clearUserTier and setUserRateLimitOverride args and role gating
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
const setUserRateLimitOverride = vi.fn();
const clearUserRateLimitOverride = vi.fn();

// Mock API service - factory must be self-contained (vi.mock is hoisted)
vi.mock('../../services/api', () => ({
  adminApi: {
    getUserRateLimit: (...args: unknown[]) => getUserRateLimit(...args),
    getUserActivity: (...args: unknown[]) => getUserActivity(...args),
    getUserUsage: (...args: unknown[]) => getUserUsage(...args),
    getUserAdminProfile: (...args: unknown[]) => getUserAdminProfile(...args),
    setUserTier: (...args: unknown[]) => setUserTier(...args),
    clearUserTier: (...args: unknown[]) => clearUserTier(...args),
    setUserRateLimitOverride: (...args: unknown[]) => setUserRateLimitOverride(...args),
    clearUserRateLimitOverride: (...args: unknown[]) => clearUserRateLimitOverride(...args),
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
    expect(screen.getByText('Monthly Usage')).toBeInTheDocument();
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
        monthly: { limit: 3000, used: 50, remaining: 2950 },
      },
      reset_times: {
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
      installed_agents: [],
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

describe('UserDetailDrawer rate-limit override', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRole = 'super_admin';
    getUserRateLimit.mockResolvedValue({
      user_id: 'user-42',
      tier: 'starter',
      rate_limits: {
        monthly: { limit: 10000, used: 1234, remaining: 8766 },
      },
      reset_times: {
        monthly_reset: '2026-10-01T00:00:00Z',
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
      from: '2026-09-01T00:00:00Z',
      by_model: [],
      total_cost_usd: 0,
      daily: [],
    });
    getUserAdminProfile.mockResolvedValue({
      user_id: 'user-42',
      coaching_persona: 'supportive_coach',
      default_coach_id: null,
      installed_agents: [],
      joined_groups: [],
    });
    setUserRateLimitOverride.mockResolvedValue(undefined);
    clearUserRateLimitOverride.mockResolvedValue({ removed: true });
  });

  it('shows the monthly budget and no daily one', async () => {
    await renderAndSettle();

    expect(screen.getByText('1,234 / 10,000')).toBeInTheDocument();
    expect(screen.queryByText('Daily Usage')).not.toBeInTheDocument();
  });

  it('sends a monthly limit and note, and never a daily one', async () => {
    await renderAndSettle();

    fireEvent.click(screen.getByText('Override limits for this user…'));
    expect(screen.queryByText('Daily limit')).not.toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText('e.g. 3000, or blank for unlimited'), {
      target: { value: '500' },
    });
    fireEvent.change(screen.getByPlaceholderText('Why this override exists'), {
      target: { value: 'pilot cap' },
    });
    fireEvent.click(screen.getByText('Save override'));

    await waitFor(() => {
      expect(setUserRateLimitOverride).toHaveBeenCalledTimes(1);
    });
    expect(setUserRateLimitOverride).toHaveBeenCalledWith('user-42', {
      monthly_limit: 500,
      note: 'pilot cap',
    });
  });

  it('sends a blank monthly limit as null, which lifts the ceiling', async () => {
    await renderAndSettle();

    fireEvent.click(screen.getByText('Override limits for this user…'));
    fireEvent.click(screen.getByText('Save override'));

    await waitFor(() => {
      expect(setUserRateLimitOverride).toHaveBeenCalledWith('user-42', {
        monthly_limit: null,
        note: null,
      });
    });
  });

  it('refuses a zero monthly limit without calling the API', async () => {
    await renderAndSettle();

    fireEvent.click(screen.getByText('Override limits for this user…'));
    fireEvent.change(screen.getByPlaceholderText('e.g. 3000, or blank for unlimited'), {
      target: { value: '0' },
    });
    fireEvent.click(screen.getByText('Save override'));

    await waitFor(() => {
      expect(
        screen.getByText('The monthly limit must be a positive integer or blank for unlimited'),
      ).toBeInTheDocument();
    });
    expect(setUserRateLimitOverride).not.toHaveBeenCalled();
  });

  it('edits an active override from its monthly limit and clears it', async () => {
    getUserRateLimit.mockResolvedValue({
      user_id: 'user-42',
      tier: 'starter',
      rate_limits: {
        monthly: { limit: 500, used: 20, remaining: 480 },
      },
      reset_times: {
        monthly_reset: '2026-10-01T00:00:00Z',
      },
      override_active: true,
      override_note: 'pilot cap',
    });
    await renderAndSettle();

    expect(screen.getByText('Per-user override active')).toBeInTheDocument();
    expect(screen.getByText('20 / 500')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Edit'));
    expect(
      (screen.getByPlaceholderText('e.g. 3000, or blank for unlimited') as HTMLInputElement).value,
    ).toBe('500');

    // Two "Clear override" buttons can exist (tier and rate limit); the
    // rate-limit one is inside the override editor.
    const clearButtons = screen.getAllByText('Clear override');
    fireEvent.click(clearButtons[clearButtons.length - 1]);
    await waitFor(() => {
      expect(clearUserRateLimitOverride).toHaveBeenCalledWith('user-42');
    });
  });
});
