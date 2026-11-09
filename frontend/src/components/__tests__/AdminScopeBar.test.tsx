// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The shared admin scope control states the server's scope and offers the picker to a super admin alone
// ABOUTME: Picking a tenant changes what every scoped read asks for; a plain admin's requests never name a tenant

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, renderHook } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AdminScopeBar from '../AdminScopeBar';
import { useAdminScope } from '../../hooks/useAdminScope';

const role = vi.hoisted(() => ({ current: 'super_admin' }));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { role: role.current } }),
}));

vi.mock('../../services/api/admin', () => ({
  adminApi: { listScopeTenants: vi.fn() },
}));

const { adminApi } = await import('../../services/api/admin');

const OTHER = '22222222-2222-4222-8222-222222222222';

function renderBar(scope: Parameters<typeof AdminScopeBar>[0]['scope']) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <AdminScopeBar scope={scope} />
    </QueryClientProvider>,
  );
}

describe('AdminScopeBar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    role.current = 'super_admin';
    vi.mocked(adminApi.listScopeTenants).mockResolvedValue([
      { id: OTHER, name: 'Alice Workspace', slug: 'alice' },
    ]);
  });

  it('defaults a super admin to every tenant and offers each tenant', async () => {
    renderBar({ kind: 'all_tenants' });
    expect(screen.getByTestId('admin-scope-statement')).toHaveTextContent(
      'Showing every tenant on the platform.',
    );
    const select = screen.getByLabelText('Tenant scope');
    expect(select).toHaveValue('all');
    await screen.findByRole('option', { name: 'Alice Workspace (alice)' });
  });

  it('names the tenant the server reports, not the one the client asked for', async () => {
    renderBar({ kind: 'tenant', tenant_id: OTHER });
    await waitFor(() => {
      expect(screen.getByTestId('admin-scope-statement')).toHaveTextContent(
        'Showing one tenant: Alice Workspace.',
      );
    });
  });

  it('picking a tenant changes what every reader of the hook requests', async () => {
    const { result } = renderHook(() => useAdminScope());
    expect(result.current.requested).toBe('all');
    renderBar({ kind: 'all_tenants' });
    await screen.findByRole('option', { name: 'Alice Workspace (alice)' });

    fireEvent.change(screen.getByLabelText('Tenant scope'), { target: { value: OTHER } });

    await waitFor(() => {
      expect(result.current.requested).toBe(OTHER);
    });
  });

  it('keeps a pick for this page when the browser refuses to store it', async () => {
    const refused = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('over quota', 'QuotaExceededError');
    });
    try {
      const { result } = renderHook(() => useAdminScope());
      renderBar({ kind: 'all_tenants' });
      await screen.findByRole('option', { name: 'Alice Workspace (alice)' });

      fireEvent.change(screen.getByLabelText('Tenant scope'), { target: { value: OTHER } });

      await waitFor(() => {
        expect(result.current.requested).toBe(OTHER);
      });
      expect(screen.getByLabelText('Tenant scope')).toHaveValue(OTHER);
    } finally {
      refused.mockRestore();
    }
    // Storage answers again: the next pick clears the held one for the next test.
    fireEvent.change(screen.getByLabelText('Tenant scope'), { target: { value: 'all' } });
  });

  it('gives a plain admin the statement, no picker, and no tenant list request', () => {
    role.current = 'admin';
    renderBar({ kind: 'tenant', tenant_id: 'mine' });
    expect(screen.getByTestId('admin-scope-statement')).toHaveTextContent(
      'Showing your tenant only.',
    );
    expect(screen.queryByLabelText('Tenant scope')).toBeNull();
    expect(adminApi.listScopeTenants).not.toHaveBeenCalled();
  });

  it('never lets a choice stored by a super admin reach a plain admin request', () => {
    localStorage.setItem('pierre.admin_scope', OTHER);
    role.current = 'admin';
    const { result } = renderHook(() => useAdminScope());
    expect(result.current.requested).toBeUndefined();
  });
});
