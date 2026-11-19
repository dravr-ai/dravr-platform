// ABOUTME: Tests for the UserDetailDrawer tier override, group coach grant, monthly rate-limit override, impersonation, suspend offers and admin role
// ABOUTME: Mocks adminApi + useAuth; asserts setUserTier/clearUserTier, setUserManagesRoster, setUserRateLimitOverride args; an admin is never impersonated; Suspend is offered on every active or pending account but the viewer's own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import UserDetailDrawer from '../UserDetailDrawer';
import type { User } from '../../types/api';
import { SUSPEND_OFFER_CASES } from './suspendOfferCases';

const getUserRateLimit = vi.fn();
const getUserActivity = vi.fn();
const getUserUsage = vi.fn();
const getUserAdminProfile = vi.fn();
const setUserTier = vi.fn();
const clearUserTier = vi.fn();
const setUserManagesRoster = vi.fn();
const setUserRateLimitOverride = vi.fn();
const clearUserRateLimitOverride = vi.fn();
const setUserAdminRole = vi.fn();

// Mock API service - factory must be self-contained (vi.mock is hoisted)
vi.mock('../../services/api', () => ({
  adminApi: {
    getUserRateLimit: (...args: unknown[]) => getUserRateLimit(...args),
    getUserActivity: (...args: unknown[]) => getUserActivity(...args),
    getUserUsage: (...args: unknown[]) => getUserUsage(...args),
    getUserAdminProfile: (...args: unknown[]) => getUserAdminProfile(...args),
    setUserTier: (...args: unknown[]) => setUserTier(...args),
    clearUserTier: (...args: unknown[]) => clearUserTier(...args),
    setUserManagesRoster: (...args: unknown[]) => setUserManagesRoster(...args),
    setUserRateLimitOverride: (...args: unknown[]) => setUserRateLimitOverride(...args),
    clearUserRateLimitOverride: (...args: unknown[]) => clearUserRateLimitOverride(...args),
    setUserAdminRole: (...args: unknown[]) => setUserAdminRole(...args),
  },
}));

// The feature-flags panel has its own tests and its own API surface
vi.mock('../FeatureFlagsPanel', () => ({
  default: () => null,
}));

// The real reset modal: it opens over the drawer, and Escape closes it first.
vi.mock('../../hooks/useAuth', async () => {
  const { VIEWER } = await import('./suspendOfferCases');
  return { useAuth: () => ({ user: VIEWER, startImpersonation: vi.fn() }) };
});

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
      manages_roster: false,
      manages_roster_operator_grant: null,
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

  it('closes the password reset modal over it on one Escape, and itself on the next', async () => {
    const onClose = vi.fn();
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <UserDetailDrawer user={targetUser} isOpen onClose={onClose} onAction={vi.fn()} />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(screen.getByText('Monthly Usage')).toBeInTheDocument());
    expect(screen.getByRole('dialog', { name: 'User Details' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Reset Password' }));
    expect(screen.getByRole('dialog', { name: 'Reset User Password' })).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Reset User Password' })).not.toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('tells apart the rows of one model by call type', async () => {
    const row = (call_type: string, total_tokens: number) => ({
      provider: 'anthropic',
      model: 'claude-sonnet-5-5',
      call_type,
      total_tokens,
      prompt_tokens: total_tokens,
      completion_tokens: 0,
      calls: 3,
      cost_usd: 0.01,
    });
    getUserUsage.mockResolvedValue({
      user_id: 'user-42',
      from: '2026-07-01T00:00:00Z',
      by_model: [row('chat', 100), row('memory_extraction', 200)],
      total_cost_usd: 0.02,
      daily: [],
    });
    await renderAndSettle();
    expect(await screen.findByText('memory_extraction')).toBeInTheDocument();
    expect(screen.getByText('chat')).toBeInTheDocument();
    expect(screen.getAllByText('anthropic/claude-sonnet-5-5')).toHaveLength(2);
  });

  it('saves the selected tier via setUserTier for an admin', async () => {
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

  it('offers an admin to impersonate an active user, never another admin', async () => {
    const first = renderDrawer();
    expect(await screen.findByRole('button', { name: /Impersonate User/ })).toBeInTheDocument();
    first.unmount();

    renderDrawer({ ...targetUser, id: 'admin-2', is_admin: true, role: 'admin' });
    await waitFor(() => {
      expect(screen.getByText('Monthly Usage')).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /Impersonate User/ })).not.toBeInTheDocument();
  });

  it('decides who is an admin from the role, whatever the is_admin flag says', async () => {
    const admin = renderDrawer({ ...targetUser, id: 'admin-3', is_admin: false, role: 'admin' });
    await waitFor(() => {
      expect(screen.getByText('Monthly Usage')).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /Impersonate User/ })).not.toBeInTheDocument();
    admin.unmount();

    renderDrawer({ ...targetUser, is_admin: true, role: 'user' });
    expect(await screen.findByRole('button', { name: /Impersonate User/ })).toBeInTheDocument();
  });
});

// The same accounts the users row is asked about (UserManagement.suspendGate.test.tsx).
describe('UserDetailDrawer suspend offer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getUserRateLimit.mockResolvedValue({
      user_id: 'user-42',
      tier: 'starter',
      rate_limits: { monthly: { limit: 3000, used: 50, remaining: 2950 } },
      reset_times: { monthly_reset: '2026-08-01T00:00:00Z' },
      override_active: false,
      override_note: null,
    });
    getUserActivity.mockResolvedValue({ user_id: 'user-42', period_days: 30, total_requests: 0, top_tools: [] });
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
      manages_roster: false,
      manages_roster_operator_grant: null,
      installed_agents: [],
      joined_groups: [],
    });
  });

  it.each(SUSPEND_OFFER_CASES)('on $name: offered is $offered', async ({ user, offered }) => {
    const onAction = vi.fn();
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <UserDetailDrawer user={user} isOpen={true} onClose={vi.fn()} onAction={onAction} />
      </QueryClientProvider>,
    );
    await waitFor(() => {
      expect(screen.getByText('Monthly Usage')).toBeInTheDocument();
    });

    const suspend = screen.queryByRole('button', { name: 'Suspend User' });
    expect(suspend !== null).toBe(offered);
    if (suspend !== null) {
      // The drawer hands the account to the list's suspend dialog, which is
      // where the server's answer is shown.
      fireEvent.click(suspend);
      expect(onAction).toHaveBeenCalledWith(user, 'suspend');
    }
  });

  it('offers a pending account Suspend beside Approve', async () => {
    const pending = SUSPEND_OFFER_CASES.find(({ user }) => user.user_status === 'pending');
    expect(pending).toBeDefined();
    renderDrawer(pending!.user);
    await waitFor(() => {
      expect(screen.getByText('Monthly Usage')).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: 'Approve User' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Suspend User' })).toBeInTheDocument();
  });
});

describe('UserDetailDrawer rate-limit override', () => {
  beforeEach(() => {
    vi.clearAllMocks();
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

describe('UserDetailDrawer group coach grant', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getUserRateLimit.mockResolvedValue({
      user_id: 'user-42',
      tier: 'starter',
      rate_limits: { monthly: { limit: 3000, used: 0, remaining: 3000 } },
      reset_times: { monthly_reset: '2026-08-01T00:00:00Z' },
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
    setUserManagesRoster.mockResolvedValue({
      user_id: 'user-42',
      email: 'athlete@example.com',
      manages_roster: true,
      manages_roster_operator_grant: { granted_at: '2026-09-29T14:00:00Z', granted_by: 'admin-1' },
    });
  });

  function profile(managesRoster: boolean, byOperator: boolean) {
    getUserAdminProfile.mockResolvedValue({
      user_id: 'user-42',
      coaching_persona: 'supportive_coach',
      default_coach_id: null,
      manages_roster: managesRoster,
      manages_roster_operator_grant: byOperator
        ? { granted_at: '2026-09-29T14:00:00Z', granted_by: 'admin-1' }
        : null,
      installed_agents: [],
      joined_groups: [],
    });
  }

  it('grants the permission to a user without it', async () => {
    profile(false, false);
    renderDrawer();
    expect(await screen.findByTestId('roster-grant-status')).toHaveTextContent(
      'Cannot coach a group',
    );

    fireEvent.click(screen.getByRole('button', { name: 'Grant' }));

    await waitFor(() => {
      expect(setUserManagesRoster).toHaveBeenCalledWith('user-42', true);
    });
  });

  it('revokes an operator grant and names where it came from', async () => {
    profile(true, true);
    renderDrawer();
    expect(await screen.findByTestId('roster-grant-status')).toHaveTextContent(
      /^Can coach a group \(granted by an operator .*2026.*\)$/,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Revoke' }));

    await waitFor(() => {
      expect(setUserManagesRoster).toHaveBeenCalledWith('user-42', false);
    });
  });

  it('names a TrainingPeaks grant as earned by the coach account', async () => {
    profile(true, false);
    renderDrawer();
    expect(await screen.findByTestId('roster-grant-status')).toHaveTextContent(
      'Can coach a group (TrainingPeaks coach account)',
    );
  });
});

describe('UserDetailDrawer admin role', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getUserRateLimit.mockResolvedValue({
      user_id: 'user-42',
      tier: 'starter',
      rate_limits: { monthly: { limit: 3000, used: 0, remaining: 3000 } },
      reset_times: { monthly_reset: '2026-08-01T00:00:00Z' },
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
    getUserAdminProfile.mockResolvedValue(null);
  });

  it('makes a user an admin only after the change is confirmed', async () => {
    setUserAdminRole.mockResolvedValue({
      id: 'user-42',
      email: 'athlete@example.com',
      is_admin: true,
      role: 'admin',
    });
    await renderAndSettle();

    fireEvent.click(screen.getByTestId('admin-role-change'));
    expect(screen.getByTestId('admin-role-confirm')).toHaveTextContent(
      'athlete@example.com gets every admin power',
    );
    expect(setUserAdminRole).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Make admin' }));

    await waitFor(() => {
      expect(setUserAdminRole).toHaveBeenCalledWith('user-42', true);
    });
    expect(await screen.findByTestId('admin-role-change')).toHaveTextContent('Remove admin role');
  });

  it('removes the admin role from another admin', async () => {
    setUserAdminRole.mockResolvedValue({
      id: 'user-42',
      email: 'athlete@example.com',
      is_admin: false,
      role: 'user',
    });
    await renderAndSettle({ ...targetUser, role: 'admin', is_admin: true });

    fireEvent.click(screen.getByTestId('admin-role-change'));
    fireEvent.click(screen.getByRole('button', { name: 'Remove admin role' }));

    await waitFor(() => {
      expect(setUserAdminRole).toHaveBeenCalledWith('user-42', false);
    });
  });

  it("shows the server's refusal, such as the last active admin", async () => {
    setUserAdminRole.mockRejectedValue({
      response: { status: 400, data: { message: 'Cannot demote the last active admin' } },
    });
    await renderAndSettle({ ...targetUser, role: 'admin', is_admin: true });

    fireEvent.click(screen.getByTestId('admin-role-change'));
    fireEvent.click(screen.getByRole('button', { name: 'Remove admin role' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Cannot demote the last active admin',
    );
  });

  it("never offers the signed-in admin a change to its own role", async () => {
    await renderAndSettle({ ...targetUser, id: 'admin-1', role: 'admin', is_admin: true });

    expect(screen.queryByTestId('admin-role-change')).not.toBeInTheDocument();
  });
});
