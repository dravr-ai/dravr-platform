// ABOUTME: Super-admin card for setting a tenant's plan (starter/professional/enterprise)
// ABOUTME: Plan gates tool availability, so saving invalidates the tenant tools queries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
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

interface TenantPlanCardProps {
  tenantId?: string;
}

export default function TenantPlanCard({ tenantId }: TenantPlanCardProps) {
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const [plan, setPlan] = useState<TenantPlan | ''>('');

  // Same tenant resolution as ToolAvailability: prop first, then login tenant
  const effectiveTenantId = tenantId || user?.tenant_id || '';

  const setPlanMutation = useMutation({
    mutationFn: (nextPlan: TenantPlan) => adminApi.setTenantPlan(effectiveTenantId, nextPlan),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminTools.tenant(effectiveTenantId) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminTools.summary(effectiveTenantId) });
    },
  });

  // Backend enforces super-admin; hide the control for everyone else
  if (user?.role !== 'super_admin' || !effectiveTenantId) {
    return null;
  }

  return (
    <Card variant="dark">
      <div className="flex flex-wrap items-center gap-3">
        <div>
          <h3 className="font-medium text-on-surface">Tenant Plan</h3>
          <p className="text-sm text-on-surface-variant mt-1">
            Sets this tenant&apos;s plan; plan-restricted tools update immediately.
          </p>
        </div>
        <div className="flex items-center gap-2 ml-auto">
          <div className="w-44">
            <Select
              aria-label="Tenant plan"
              size="sm"
              value={plan}
              onChange={(e) => setPlan(e.target.value as TenantPlan)}
              options={PLAN_OPTIONS}
              placeholder="Select plan…"
            />
          </div>
          <Button
            size="sm"
            onClick={() => {
              if (plan) {
                setPlanMutation.mutate(plan);
              }
            }}
            disabled={!plan || setPlanMutation.isPending}
          >
            {setPlanMutation.isPending ? 'Saving…' : 'Save'}
          </Button>
        </div>
      </div>
      {setPlanMutation.isError && (
        <p className="text-sm text-red-400 mt-2">Failed to set tenant plan. Please try again.</p>
      )}
      {setPlanMutation.isSuccess && (
        <p className="text-sm text-green-400 mt-2">
          Plan set to {setPlanMutation.data.plan}.
        </p>
      )}
    </Card>
  );
}
