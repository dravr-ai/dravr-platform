// ABOUTME: Admin UI for runtime feature flags — tenant defaults or per-user overrides
// ABOUTME: Reused by AdminSettings (tenant scope) and UserDetailDrawer (user scope)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import type { FeatureFlagRow, KnownFeatureFlag } from '../services/api/admin';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Card } from './ui';

type Scope =
  | { kind: 'tenant'; id: string }
  | { kind: 'user'; id: string };

interface FeatureFlagsPanelProps {
  scope: Scope;
}

interface FlagState {
  key: string;
  description: string;
  defaultEnabled: boolean;
  storedRow: FeatureFlagRow | null;
}

function mergeRowsWithKnown(rows: FeatureFlagRow[], known: KnownFeatureFlag[]): FlagState[] {
  const byKey = new Map(rows.map((r) => [r.feature_key, r] as const));
  return known.map((k) => ({
    key: k.key,
    description: k.description,
    defaultEnabled: k.default_enabled,
    storedRow: byKey.get(k.key) ?? null,
  }));
}

export default function FeatureFlagsPanel({ scope }: FeatureFlagsPanelProps) {
  const queryClient = useQueryClient();

  const queryKey =
    scope.kind === 'tenant'
      ? QUERY_KEYS.featureFlags.tenantDefaults(scope.id)
      : QUERY_KEYS.featureFlags.userOverrides(scope.id);

  const { data, isLoading, error } = useQuery({
    queryKey,
    queryFn: () =>
      scope.kind === 'tenant'
        ? adminApi.getTenantFeatureFlags(scope.id)
        : adminApi.getUserFeatureOverrides(scope.id),
    retry: 1,
  });

  // After a write, also invalidate the calling admin's own
  // `/api/me/features` so toggling a flag while sitting on their own user
  // Settings tab updates immediately instead of waiting for staleTime.
  const invalidateAfterWrite = () => {
    queryClient.invalidateQueries({ queryKey });
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.featureFlags.self() });
  };

  const setMutation = useMutation({
    mutationFn: async (vars: { key: string; enabled: boolean }) =>
      scope.kind === 'tenant'
        ? adminApi.setTenantFeatureFlag(scope.id, vars.key, vars.enabled)
        : adminApi.setUserFeatureOverride(scope.id, vars.key, vars.enabled),
    onSuccess: invalidateAfterWrite,
  });

  const clearMutation = useMutation({
    mutationFn: async (key: string) =>
      scope.kind === 'tenant'
        ? adminApi.clearTenantFeatureFlag(scope.id, key)
        : adminApi.clearUserFeatureOverride(scope.id, key),
    onSuccess: invalidateAfterWrite,
  });

  const flags = useMemo<FlagState[]>(() => {
    if (!data) return [];
    return mergeRowsWithKnown(data.rows, data.known);
  }, [data]);

  const heading = scope.kind === 'tenant' ? 'Feature Flags' : 'Feature Flag Overrides';
  const subtitle =
    scope.kind === 'tenant'
      ? 'Toggle features for everyone in this tenant. Per-user overrides win when set.'
      : 'Per-user overrides win over the tenant default. "Inherit" removes the override.';

  return (
    <Card variant="dark">
      <h2 className="text-lg font-semibold text-on-surface mb-1">{heading}</h2>
      <p className="text-sm text-on-surface-variant mb-4">{subtitle}</p>

      {isLoading && (
        <div className="space-y-3 animate-pulse">
          {[1, 2].map((k) => (
            <div key={k} className="h-12 bg-surface-container-high rounded" />
          ))}
        </div>
      )}

      {error != null && !isLoading && (
        <p className="text-sm text-error">
          Failed to load flags: {error instanceof Error ? error.message : String(error)}
        </p>
      )}

      {!isLoading && !error && flags.length === 0 && (
        <p className="text-sm text-on-surface-variant">No feature flags registered.</p>
      )}

      <div className="space-y-3">
        {flags.map((flag) => {
          const effective = flag.storedRow?.enabled ?? flag.defaultEnabled;
          return (
            <div
              key={flag.key}
              className="flex items-start justify-between gap-4 p-3 rounded-md bg-surface-container-high"
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <code className="font-mono text-sm text-on-surface">{flag.key}</code>
                  <span
                    className={
                      'text-xs px-2 py-0.5 rounded-full ' +
                      (effective
                        ? 'bg-success text-success'
                        : 'bg-error text-error')
                    }
                  >
                    {effective ? 'enabled' : 'disabled'}
                  </span>
                  {flag.storedRow == null && (
                    <span className="text-xs text-on-surface-variant italic">(default)</span>
                  )}
                </div>
                <p className="text-sm text-on-surface-variant mt-1">{flag.description}</p>
                {flag.storedRow != null && (
                  <p className="text-xs text-on-surface-variant mt-1">
                    Set {new Date(flag.storedRow.updated_at).toLocaleString()}
                  </p>
                )}
              </div>
              <div className="flex flex-col gap-2 shrink-0">
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => setMutation.mutate({ key: flag.key, enabled: true })}
                    disabled={setMutation.isPending}
                    className={
                      'px-2 py-1 text-xs rounded ' +
                      (effective
                        ? 'bg-success text-white'
                        : 'bg-surface-container text-on-surface-variant hover:bg-surface-container-high')
                    }
                  >
                    Enable
                  </button>
                  <button
                    type="button"
                    onClick={() => setMutation.mutate({ key: flag.key, enabled: false })}
                    disabled={setMutation.isPending}
                    className={
                      'px-2 py-1 text-xs rounded ' +
                      (!effective && flag.storedRow != null
                        ? 'bg-error text-white'
                        : 'bg-surface-container text-on-surface-variant hover:bg-surface-container-high')
                    }
                  >
                    Disable
                  </button>
                </div>
                {flag.storedRow != null && (
                  <button
                    type="button"
                    onClick={() => clearMutation.mutate(flag.key)}
                    disabled={clearMutation.isPending}
                    className="px-2 py-1 text-xs rounded bg-surface-container text-on-surface-variant hover:bg-surface-container-high"
                  >
                    {scope.kind === 'tenant' ? 'Reset to default' : 'Inherit from tenant'}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {setMutation.error != null && (
        <p className="text-sm text-error mt-3">
          Update failed: {setMutation.error instanceof Error ? setMutation.error.message : 'unknown'}
        </p>
      )}
    </Card>
  );
}
