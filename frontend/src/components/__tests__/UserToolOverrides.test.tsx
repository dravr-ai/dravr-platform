// ABOUTME: Tests for UserToolOverrides — per-user tool panel with source badges and overrides
// ABOUTME: Mocks adminApi; asserts tool rendering, toggle args, global-disable lockout, reset affordance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import UserToolOverrides from '../UserToolOverrides';
import type { UserEffectiveTool } from '../../services/api/admin';
import type { User } from '../../types/api';

const getAllUsers = vi.fn();
const getUserTools = vi.fn();
const setUserToolOverride = vi.fn();
const removeUserToolOverride = vi.fn();

// Mock API service - factory must be self-contained (vi.mock is hoisted)
vi.mock('../../services/api', () => ({
  adminApi: {
    getAllUsers: (...args: unknown[]) => getAllUsers(...args),
    getUserTools: (...args: unknown[]) => getUserTools(...args),
    setUserToolOverride: (...args: unknown[]) => setUserToolOverride(...args),
    removeUserToolOverride: (...args: unknown[]) => removeUserToolOverride(...args),
  },
}));

const athlete: User = {
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

const sampleTools: UserEffectiveTool[] = [
  {
    tool_name: 'get_activities',
    display_name: 'Get Activities',
    description: 'List recent activities from connected providers',
    category: 'fitness',
    is_enabled: true,
    source: 'default',
    min_plan: 'starter',
  },
  {
    tool_name: 'analyze_activity',
    display_name: 'Analyze Activity',
    description: 'Deep analysis of a single activity',
    category: 'analysis',
    is_enabled: false,
    source: 'user_override',
    min_plan: 'starter',
  },
  {
    tool_name: 'set_configuration',
    display_name: 'Set Configuration',
    description: 'Modify runtime configuration parameters',
    category: 'configuration',
    is_enabled: false,
    source: 'global_disabled',
    min_plan: 'starter',
  },
  {
    tool_name: 'admin_assign_coach',
    display_name: 'Assign Coach',
    description: 'Assign a system coach to a specific user (admin only)',
    category: 'admin',
    is_enabled: true,
    source: 'default',
    min_plan: 'starter',
  },
];

function renderPanel() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <UserToolOverrides />
    </QueryClientProvider>,
  );
}

async function renderAndSelectUser() {
  renderPanel();
  const userRow = await screen.findByText('athlete@example.com');
  fireEvent.click(userRow);
  await waitFor(() => {
    expect(screen.getByText('Get Activities')).toBeInTheDocument();
  });
}

describe('UserToolOverrides', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getAllUsers.mockResolvedValue([athlete]);
    getUserTools.mockResolvedValue({ success: true, message: 'ok', data: sampleTools });
    setUserToolOverride.mockResolvedValue({
      success: true,
      message: 'ok',
      data: {
        user_id: 'user-42',
        tool_name: 'get_activities',
        is_enabled: false,
        set_by: 'admin-1',
        reason: null,
        created_at: '2026-07-16T00:00:00Z',
        updated_at: '2026-07-16T00:00:00Z',
      },
    });
    removeUserToolOverride.mockResolvedValue({ success: true, message: 'ok' });
  });

  it('renders the selected user tools grouped by category with source badges', async () => {
    await renderAndSelectUser();

    expect(getUserTools).toHaveBeenCalledWith('user-42');

    // Category section headings
    expect(screen.getByText('fitness')).toBeInTheDocument();
    expect(screen.getByText('analysis')).toBeInTheDocument();
    expect(screen.getByText('configuration')).toBeInTheDocument();

    // Tool display names + machine names
    expect(screen.getByText('Get Activities')).toBeInTheDocument();
    expect(screen.getByText('get_activities')).toBeInTheDocument();
    expect(screen.getByText('Analyze Activity')).toBeInTheDocument();
    expect(screen.getByText('Set Configuration')).toBeInTheDocument();

    // Color-coded source badges
    expect(screen.getByText('Default')).toBeInTheDocument();
    expect(screen.getByText('User Override')).toBeInTheDocument();
    expect(screen.getByText('Global Block')).toBeInTheDocument();

    // Admin-category tools are role-gated (stripped from a non-admin's
    // tools/list by the registry), so the per-user panel must not offer
    // them — showing "Assign Coach: enabled" for a regular user would lie.
    expect(screen.queryByText('admin')).not.toBeInTheDocument();
    expect(screen.queryByText('Assign Coach')).not.toBeInTheDocument();
    expect(screen.queryByText('admin_assign_coach')).not.toBeInTheDocument();
  });

  it('toggling a tool calls setUserToolOverride with the flipped enablement', async () => {
    await renderAndSelectUser();

    // Enabled default tool → toggle should disable it
    fireEvent.click(screen.getByRole('switch', { name: 'Toggle Get Activities' }));
    await waitFor(() => {
      expect(setUserToolOverride).toHaveBeenCalledWith('user-42', 'get_activities', false);
    });

    // Disabled user-override tool → toggle should enable it
    fireEvent.click(screen.getByRole('switch', { name: 'Toggle Analyze Activity' }));
    await waitFor(() => {
      expect(setUserToolOverride).toHaveBeenCalledWith('user-42', 'analyze_activity', true);
    });
  });

  it('disables the toggle for globally disabled tools', async () => {
    await renderAndSelectUser();

    const globalToggle = screen.getByRole('switch', { name: 'Toggle Set Configuration' });
    expect(globalToggle).toBeDisabled();

    fireEvent.click(globalToggle);
    expect(setUserToolOverride).not.toHaveBeenCalled();
  });

  it('shows the reset affordance only for user_override rows and removes the override', async () => {
    await renderAndSelectUser();

    // Exactly one row (analyze_activity) carries a per-user override
    const resetButtons = screen.getAllByTitle(/remove override/i);
    expect(resetButtons).toHaveLength(1);

    fireEvent.click(resetButtons[0]);
    await waitFor(() => {
      expect(removeUserToolOverride).toHaveBeenCalledWith('user-42', 'analyze_activity');
    });
  });
});
