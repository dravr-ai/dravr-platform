// ABOUTME: Phase B Sprint C6 — admin tab monitoring the memory extraction worker
// ABOUTME: Derives health from user_facts aggregates (total, 24h/7d activity, per-kind breakdown)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  adminApi,
  type MemoryWorkerMetricsResponse,
  type UserFactMetrics,
} from '../services/api/admin';
import { Card, Button } from './ui';
import { useAuth } from '../hooks/useAuth';

const KIND_LABELS: Record<string, string> = {
  preference: 'Preferences',
  physiology: 'Physiology',
  injury: 'Injuries',
  goal: 'Goals',
  schedule: 'Schedules',
  equipment: 'Equipment',
  other: 'Other',
};

function humanizeKind(kind: string): string {
  return KIND_LABELS[kind] ?? kind.charAt(0).toUpperCase() + kind.slice(1);
}

function formatTimestamp(iso: string | null): string {
  if (!iso) {
    return 'No facts yet';
  }
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function relativeFromNow(iso: string | null): string {
  if (!iso) {
    return '—';
  }
  const then = Date.parse(iso);
  if (Number.isNaN(then)) {
    return iso;
  }
  const diffMs = Date.now() - then;
  if (diffMs < 0) {
    return 'just now';
  }
  const minutes = Math.round(diffMs / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

function healthStatus(metrics: UserFactMetrics): {
  label: string;
  variant: 'healthy' | 'warning' | 'idle' | 'stalled';
  message: string;
} {
  if (metrics.total_facts === 0) {
    return {
      label: 'Idle',
      variant: 'idle',
      message:
        'No facts have been extracted yet for this tenant. Run a few coaching conversations to seed the memory store.',
    };
  }
  if (metrics.facts_last_24h === 0 && metrics.facts_last_7d === 0) {
    return {
      label: 'Stalled',
      variant: 'stalled',
      message:
        'The extractor has produced nothing for 7+ days despite having prior facts. Check worker errors and LLM credentials.',
    };
  }
  if (metrics.facts_last_24h === 0) {
    return {
      label: 'Warming',
      variant: 'warning',
      message:
        'No extractions in the last 24 hours but activity within the past week. Expected during low-usage periods.',
    };
  }
  return {
    label: 'Healthy',
    variant: 'healthy',
    message: 'Extractions are landing within the last 24 hours.',
  };
}

const STATUS_CLASS: Record<ReturnType<typeof healthStatus>['variant'], string> = {
  healthy: 'bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300',
  warning: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/40 dark:text-yellow-300',
  idle: 'bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-200',
  stalled: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300',
};

export default function MemoryExtractionMonitorTab() {
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery<
    MemoryWorkerMetricsResponse
  >({
    queryKey: ['admin', 'memory-worker-metrics', tenantId] as const,
    queryFn: () => adminApi.getMemoryWorkerMetrics(tenantId),
    enabled: Boolean(tenantId),
    refetchInterval: 60_000,
  });

  const metrics = data?.metrics;

  const kindRows = useMemo(() => {
    if (!metrics) {
      return [] as { kind: string; count: number }[];
    }
    return Object.entries(metrics.facts_by_kind)
      .map(([kind, count]) => ({ kind, count }))
      .sort((a, b) => b.count - a.count);
  }, [metrics]);

  if (!tenantId) {
    return (
      <Card className="p-6">
        <p className="text-sm text-gray-500 dark:text-gray-400">
          Tenant id not available on your session. Reload the page and try again.
        </p>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-gray-900 dark:text-white">
              Memory extraction worker
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Health of the fire-and-forget extraction pipeline derived from the{' '}
              <code className="rounded bg-gray-100 px-1 py-0.5 text-xs dark:bg-gray-800">
                user_facts
              </code>{' '}
              table. When the 24-hour counter sits at zero despite active
              conversations, the worker is broken.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>
      </Card>

      {isLoading ? (
        <Card className="p-12">
          <div className="flex justify-center">
            <div className="pierre-spinner" />
          </div>
        </Card>
      ) : isError ? (
        <Card className="p-6">
          <p className="text-sm text-red-600 dark:text-red-400">
            Failed to load memory worker metrics:{' '}
            {error instanceof Error ? error.message : String(error)}
          </p>
        </Card>
      ) : metrics ? (
        <>
          <Card className="p-6">
            {(() => {
              const status = healthStatus(metrics);
              return (
                <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                  <div className="flex items-center gap-3">
                    <span
                      className={`rounded-full px-3 py-1 text-sm font-semibold ${STATUS_CLASS[status.variant]}`}
                    >
                      {status.label}
                    </span>
                    <p className="text-sm text-gray-600 dark:text-gray-300">{status.message}</p>
                  </div>
                  <div className="text-sm text-gray-500 dark:text-gray-400">
                    Last activity{' '}
                    <span className="font-medium text-gray-900 dark:text-gray-100">
                      {relativeFromNow(metrics.newest_updated_at)}
                    </span>
                    <span className="ml-2 text-xs text-gray-400 dark:text-gray-500">
                      ({formatTimestamp(metrics.newest_updated_at)})
                    </span>
                  </div>
                </div>
              );
            })()}
          </Card>

          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            <Card className="p-4">
              <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
                Total facts
              </div>
              <div className="mt-1 text-3xl font-semibold text-gray-900 dark:text-white">
                {metrics.total_facts.toLocaleString()}
              </div>
            </Card>
            <Card className="p-4">
              <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
                Last 24 hours
              </div>
              <div className="mt-1 text-3xl font-semibold text-gray-900 dark:text-white">
                {metrics.facts_last_24h.toLocaleString()}
              </div>
            </Card>
            <Card className="p-4">
              <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
                Last 7 days
              </div>
              <div className="mt-1 text-3xl font-semibold text-gray-900 dark:text-white">
                {metrics.facts_last_7d.toLocaleString()}
              </div>
            </Card>
            <Card className="p-4">
              <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
                Distinct users
              </div>
              <div className="mt-1 text-3xl font-semibold text-gray-900 dark:text-white">
                {metrics.distinct_users.toLocaleString()}
              </div>
            </Card>
          </div>

          <Card className="overflow-hidden">
            <div className="border-b border-gray-200 px-6 py-3 dark:border-gray-700">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-white">
                Breakdown by fact kind
              </h3>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                Extraction distribution across semantic categories — heavy skew toward{' '}
                <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">other</code> usually
                means the extractor prompt needs tuning.
              </p>
            </div>
            {kindRows.length === 0 ? (
              <div className="p-6 text-sm text-gray-500 dark:text-gray-400">
                No facts stored yet.
              </div>
            ) : (
              <ul className="divide-y divide-gray-200 dark:divide-gray-700">
                {kindRows.map((row) => {
                  const pct =
                    metrics.total_facts === 0
                      ? 0
                      : Math.round((row.count / metrics.total_facts) * 100);
                  return (
                    <li key={row.kind} className="flex items-center gap-4 px-6 py-3">
                      <div className="w-32 text-sm font-medium text-gray-900 dark:text-gray-100">
                        {humanizeKind(row.kind)}
                      </div>
                      <div className="flex-1">
                        <div className="h-2 overflow-hidden rounded-full bg-gray-100 dark:bg-gray-800">
                          <div
                            className="h-full bg-pierre-violet"
                            style={{ width: `${pct}%` }}
                          />
                        </div>
                      </div>
                      <div className="w-24 text-right text-sm text-gray-600 dark:text-gray-300">
                        {row.count.toLocaleString()}{' '}
                        <span className="text-xs text-gray-400 dark:text-gray-500">({pct}%)</span>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </Card>
        </>
      ) : null}
    </div>
  );
}
