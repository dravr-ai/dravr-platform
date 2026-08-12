// ABOUTME: Super-admin card for setting a tenant's plan (starter/professional/enterprise)
// ABOUTME: Plan gates tool availability, so saving invalidates the tenant tools queries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import { Card, Button, Select } from './ui';
import { useAuth } from '../hooks/useAuth';
import { QUERY_KEYS } from '../constants/queryKeys';

type TenantPlan = 'starter' | 'professional' | 'enterprise';

const PLAN_OPTIONS = [
  { value: 'starter', label: 'Starter' },
  { value: 'professional', label: 'Professional' },
  { value: 'enterprise', label: 'Enterprise' },
];

// Per-tenant plan query key (per-tenant tool keys live in QUERY_KEYS.adminTools)
const tenantPlanQueryKey = (tenantId: string) => ['tenant-plan', tenantId] as const;

interface TenantPlanCardProps {
  tenantId?: string;
}

export default function TenantPlanCard({ tenantId }: TenantPlanCardProps) {
  const { user } = useAuth();
  const queryClient = useQueryClient();
  // '' = no explicit selection yet; the select then tracks the fetched current plan
  const [selectedPlan, setSelectedPlan] = useState<TenantPlan | ''>('');

  // Same tenant resolution as ToolAvailability: prop first, then login tenant
  const effectiveTenantId = tenantId || user?.tenant_id || '';
  const isSuperAdmin = user?.role === 'super_admin';

  const { data: planData, isLoading: planLoading } = useQuery({
    queryKey: tenantPlanQueryKey(effectiveTenantId),
    queryFn: () => adminApi.getTenantPlan(effectiveTenantId),
    retry: 1,
    enabled: isSuperAdmin && !!effectiveTenantId,
  });
  const currentPlan = planData?.plan;

  const setPlanMutation = useMutation({
    mutationFn: (nextPlan: TenantPlan) => adminApi.setTenantPlan(effectiveTenantId, nextPlan),
    onSuccess: () => {
      // Track the refetched server plan again so Save re-disables after a save
      setSelectedPlan('');
      queryClient.invalidateQueries({ queryKey: tenantPlanQueryKey(effectiveTenantId) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminTools.tenant(effectiveTenantId) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminTools.summary(effectiveTenantId) });
    },
  });

  // Backend enforces super-admin; hide the control for everyone else
  if (!isSuperAdmin || !effectiveTenantId) {
    return null;
  }

  const shownPlan = selectedPlan || currentPlan || '';
  const canSave = shownPlan !== '' && shownPlan !== currentPlan && !setPlanMutation.isPending;

  return (
    <Card variant="dark">
      <div className="flex flex-wrap items-center gap-3">
        <div>
          <h3 className="font-medium text-on-surface">Tenant Plan</h3>
          <p className="text-sm text-on-surface-variant mt-1">
            Sets this tenant&apos;s plan; plan-restricted tools update immediately.
          </p>
          {currentPlan && (
            <p className="text-sm text-on-surface-variant mt-1">Current: {currentPlan}</p>
          )}
        </div>
        <div className="flex items-center gap-2 ml-auto">
          <div className="w-44">
            <Select
              aria-label="Tenant plan"
              size="sm"
              value={shownPlan}
              onChange={(e) => setSelectedPlan(e.target.value as TenantPlan)}
              options={PLAN_OPTIONS}
              disabled={planLoading}
            />
          </div>
          <Button
            size="sm"
            onClick={() => {
              if (shownPlan && shownPlan !== currentPlan) {
                setPlanMutation.mutate(shownPlan);
              }
            }}
            disabled={!canSave}
          >
            {setPlanMutation.isPending ? 'Saving…' : 'Save'}
          </Button>
        </div>
      </div>
      {setPlanMutation.isError && (
        <p className="text-sm text-error mt-2">Failed to set tenant plan. Please try again.</p>
      )}
      {setPlanMutation.isSuccess && (
        <p className="text-sm text-success mt-2">
          Plan set to {setPlanMutation.data.plan}.
        </p>
      )}
    </Card>
  );
}
