// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: States which tenants a scoped admin screen is showing and lets a super admin pick one
// ABOUTME: The statement reads the server's reported scope, so it cannot claim more than was read

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { adminApi } from '../services/api/admin';
import { ALL_TENANTS, useAdminScope } from '../hooks/useAdminScope';
import type { AdminScope } from '../hooks/useAdminScope';
import { Select } from './ui';

export default function AdminScopeBar({ scope }: { scope: AdminScope | undefined }) {
  const { isSuperAdmin, requested, pick } = useAdminScope();
  const { data: tenants } = useQuery({
    queryKey: ['admin', 'scope-tenants'] as const,
    queryFn: () => adminApi.listScopeTenants(),
    enabled: isSuperAdmin,
  });

  const options = useMemo(() => {
    const listed = (tenants ?? []).map((tenant) => ({
      value: tenant.id,
      label: `${tenant.name} (${tenant.slug})`,
    }));
    const picked = requested && requested !== ALL_TENANTS ? requested : null;
    // A choice stored in this browser can name a tenant since deleted, or one
    // the list has not delivered yet; the select must still show what is read.
    const missing = picked && !listed.some((option) => option.value === picked);
    return [
      { value: ALL_TENANTS, label: 'All tenants' },
      ...(missing ? [{ value: picked, label: picked }] : []),
      ...listed,
    ];
  }, [tenants, requested]);

  const statement = (() => {
    if (!scope) return null;
    if (scope.kind === 'all_tenants') return 'Showing every tenant on the platform.';
    if (!isSuperAdmin) return 'Showing your tenant only.';
    const tenant = tenants?.find((row) => row.id === scope.tenant_id);
    return `Showing one tenant: ${tenant?.name ?? scope.tenant_id}.`;
  })();

  return (
    <div
      className="flex flex-col gap-2 md:flex-row md:items-center md:justify-between"
      data-testid="admin-scope-bar"
    >
      <p className="text-sm text-on-surface-variant" data-testid="admin-scope-statement">
        {statement}
      </p>
      {isSuperAdmin && (
        <div className="md:w-80">
          <Select
            aria-label="Tenant scope"
            size="sm"
            options={options}
            value={requested}
            onChange={(event) => pick(event.target.value)}
          />
        </div>
      )}
    </div>
  );
}
