// ABOUTME: Tests the Auto-Approve Registrations toggle in AdminSettings
// ABOUTME: Asserts the toggle is locked and explained when the server env overrides the setting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AdminSettings from '../AdminSettings';

vi.mock('../../services/api', () => ({
  adminApi: {
    getAutoApprovalSetting: vi.fn(),
    updateAutoApprovalSetting: vi.fn(),
    getSignupSetting: vi.fn(() =>
      Promise.resolve({
        open: true,
        daily_limit: 0,
        activate_on_verified_email: false,
        counted: 0,
        counted_since: '2026-10-22T08:00:00Z',
        turnstile_configured: false,
        description: 'Sign-up guards',
      }),
    ),
    updateSignupSetting: vi.fn(),
    updateConfig: vi.fn(),
  },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { tenant_id: null } }),
}));

vi.mock('../../hooks/useGroups', () => ({
  useGroupPermissions: () => ({ policy: 'admins_only', isLoading: false }),
}));

vi.mock('../FeatureFlagsPanel', () => ({
  default: () => <div data-testid="feature-flags-panel" />,
}));

const { adminApi } = await import('../../services/api');

const AUTO_APPROVAL_DESCRIPTION =
  'When enabled, all new registrations are auto-approved. When disabled, only emails from auto_approve_domains are auto-approved.';

function renderSettings() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <AdminSettings />
    </QueryClientProvider>,
  );
}

describe('AdminSettings auto-approval toggle', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('locks the toggle and explains why when the server environment overrides the setting', async () => {
    const envLocked = {
      enabled: false,
      description: AUTO_APPROVAL_DESCRIPTION,
      overridden_by_env: true,
    };
    vi.mocked(adminApi.getAutoApprovalSetting).mockResolvedValue(envLocked);

    renderSettings();

    const toggle = await screen.findByRole('switch', { name: 'Auto-Approve Registrations' });
    await waitFor(() => expect(toggle).toBeDisabled());

    const lockNote = screen.getByTestId('auto-approval-env-lock');
    expect(lockNote.textContent).toContain('AUTO_APPROVE_USERS');

    fireEvent.click(toggle);
    expect(adminApi.updateAutoApprovalSetting).not.toHaveBeenCalled();
  });

  it('keeps the toggle editable and saves the flipped value when nothing overrides it', async () => {
    const dbOwned = {
      enabled: false,
      description: AUTO_APPROVAL_DESCRIPTION,
      overridden_by_env: false,
    };
    vi.mocked(adminApi.getAutoApprovalSetting).mockResolvedValue(dbOwned);
    vi.mocked(adminApi.updateAutoApprovalSetting).mockResolvedValue({
      enabled: true,
      description: AUTO_APPROVAL_DESCRIPTION,
    });

    renderSettings();

    const toggle = await screen.findByRole('switch', { name: 'Auto-Approve Registrations' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(screen.queryByTestId('auto-approval-env-lock')).toBeNull();

    fireEvent.click(toggle);
    await waitFor(() =>
      expect(adminApi.updateAutoApprovalSetting).toHaveBeenCalledWith(true),
    );
  });
});
