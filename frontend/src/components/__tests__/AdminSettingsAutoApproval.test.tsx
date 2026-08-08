// ABOUTME: Tests the Auto-Approve Registrations toggle in AdminSettings
// ABOUTME: Asserts the toggle is locked and explained when the server env overrides the setting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AdminSettings from '../AdminSettings';
import type { SocialInsightsConfig } from '../../types/api';

vi.mock('../../services/api', () => ({
  adminApi: {
    getAutoApprovalSetting: vi.fn(),
    updateAutoApprovalSetting: vi.fn(),
    getSocialInsightsConfig: vi.fn(),
    updateSocialInsightsConfig: vi.fn(),
    resetSocialInsightsConfig: vi.fn(),
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

function socialInsightsConfig(): SocialInsightsConfig {
  return {
    milestone_thresholds: { min_activities_for_milestone: 10, activity_counts: [50, 100] },
    distance_milestones: { thresholds_km: [100, 500], near_milestone_percent: 90 },
    streak_config: { lookback_days: 90, min_for_sharing: 3, milestone_days: [30, 60] },
    relevance_scoring: {
      activity_milestone_scores: {
        score_1000_plus: 100,
        score_500_999: 90,
        score_250_499: 80,
        score_100_249: 70,
        score_50_99: 60,
        score_25_49: 50,
        score_default: 40,
      },
      distance_milestone_scores: {
        score_10000_plus: 100,
        score_5000_9999: 90,
        score_2500_4999: 80,
        score_1000_2499: 70,
        score_500_999: 60,
        score_default: 50,
      },
      streak_scores: {
        score_365_plus: 100,
        score_180_364: 90,
        score_90_179: 80,
        score_60_89: 70,
        score_30_59: 60,
        score_default: 50,
      },
      pr_base_score: 75,
      training_phase_base_score: 65,
    },
    activity_fetch_limits: {
      insight_context_limit: 50,
      training_context_limit: 20,
      max_client_limit: 100,
    },
    min_relevance_score: 60,
  };
}

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
    vi.mocked(adminApi.getSocialInsightsConfig).mockResolvedValue(socialInsightsConfig());
  });

  it('locks the toggle and explains why when the server environment overrides the setting', async () => {
    const envLocked = {
      enabled: false,
      description: AUTO_APPROVAL_DESCRIPTION,
      overridden_by_env: true,
    };
    vi.mocked(adminApi.getAutoApprovalSetting).mockResolvedValue(envLocked);

    renderSettings();

    const toggle = await screen.findByRole('switch');
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

    const toggle = await screen.findByRole('switch');
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(screen.queryByTestId('auto-approval-env-lock')).toBeNull();

    fireEvent.click(toggle);
    await waitFor(() =>
      expect(adminApi.updateAutoApprovalSetting).toHaveBeenCalledWith(true),
    );
  });
});
