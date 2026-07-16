// ABOUTME: Tests for TenantPlanCard — current-plan fetch, preselection, and save gating
// ABOUTME: Mocks adminApi + useAuth; asserts preselected plan, Save disabled until changed, save args
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import TenantPlanCard from '../TenantPlanCard';

const getTenantPlan = vi.fn();
const setTenantPlan = vi.fn();

// Mock API service - factory must be self-contained (vi.mock is hoisted)
vi.mock('../../services/api', () => ({
  adminApi: {
    getTenantPlan: (...args: unknown[]) => getTenantPlan(...args),
    setTenantPlan: (...args: unknown[]) => setTenantPlan(...args),
  },
}));

// Card renders only for super_admin; pin the logged-in tenant it resolves
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'admin-1',
      email: 'admin@example.com',
      tenant_id: 'tenant-a',
      is_admin: true,
      role: 'super_admin',
      tier: 'enterprise',
      created_at: '2026-01-01T00:00:00Z',
    },
  }),
}));

function renderCard() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <TenantPlanCard />
    </QueryClientProvider>,
  );
}

describe('TenantPlanCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getTenantPlan.mockResolvedValue({ tenant_id: 'tenant-a', plan: 'professional' });
    setTenantPlan.mockResolvedValue({ tenant_id: 'tenant-a', plan: 'enterprise' });
  });

  it('fetches and preselects the current plan with Save disabled', async () => {
    renderCard();

    expect(await screen.findByText('Current: professional')).toBeInTheDocument();
    expect(getTenantPlan).toHaveBeenCalledWith('tenant-a');

    const select = screen.getByLabelText('Tenant plan') as HTMLSelectElement;
    expect(select.value).toBe('professional');
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });

  it('enables Save when the selection differs and saves the new plan', async () => {
    renderCard();
    await screen.findByText('Current: professional');

    fireEvent.change(screen.getByLabelText('Tenant plan'), {
      target: { value: 'enterprise' },
    });

    const save = screen.getByRole('button', { name: 'Save' });
    expect(save).toBeEnabled();
    fireEvent.click(save);

    await waitFor(() => {
      expect(setTenantPlan).toHaveBeenCalledWith('tenant-a', 'enterprise');
    });
    expect(await screen.findByText('Plan set to enterprise.')).toBeInTheDocument();
  });

  it('re-disables Save when the selection returns to the current plan', async () => {
    renderCard();
    await screen.findByText('Current: professional');

    const select = screen.getByLabelText('Tenant plan');
    fireEvent.change(select, { target: { value: 'starter' } });
    expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled();

    fireEvent.change(select, { target: { value: 'professional' } });
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    expect(setTenantPlan).not.toHaveBeenCalled();
  });
});
