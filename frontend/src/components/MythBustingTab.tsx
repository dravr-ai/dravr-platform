// ABOUTME: Phase D Sprint C13 — admin myth-busting tab summarizing Tier 5.5 unsupported claims
// ABOUTME: Top recurring claim texts, top offending coaches, top categories from claim_verdicts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { adminApi, type MythBustingSummary } from '../services/api/admin';
import { Card, Button } from './ui';
import { useAuth } from '../hooks/useAuth';

function formatTimestamp(iso: string | null): string {
  if (!iso) return '—';
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function humanizeCategory(cat: string): string {
  return cat.replace(/_/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
}

export default function MythBustingTab() {
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery<MythBustingSummary>({
    queryKey: ['admin', 'myth-busting', tenantId] as const,
    queryFn: () => adminApi.getMythBustingSummary(tenantId, 200),
    enabled: Boolean(tenantId),
    refetchInterval: 120_000,
  });

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
            <h2 className="text-xl font-semibold text-gray-900 dark:text-on-surface">
              Myth-busting — recurring unsupported claims
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Pattern view over the latest{' '}
              <span className="font-medium">{data?.verdicts_scanned ?? '—'}</span>{' '}
              Tier 5.5 verdicts. Highlights the recurring unsupported and
              contradicted claims, the coaches that emit them, and the
              categories most likely to need myth-busting.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Verdicts scanned
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.verdicts_scanned ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Flagged total
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.flagged_total ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Distinct claims
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.top_claims.length ?? 0}
            </div>
          </div>
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
            Failed to load myth-busting summary:{' '}
            {error instanceof Error ? error.message : String(error)}
          </p>
        </Card>
      ) : data && data.flagged_total === 0 ? (
        <Card className="p-12 text-center">
          <p className="text-gray-500 dark:text-gray-400">
            No unsupported or contradicted claims in the latest scan window.
          </p>
          <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
            Either coaches are well-behaved, or the Tier 5.5 detector
            classified everything as supported / rhetorical.
          </p>
        </Card>
      ) : data ? (
        <>
          <Card className="overflow-hidden">
            <div className="border-b border-gray-200 px-6 py-3 dark:border-gray-700">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-on-surface">
                Top recurring claims
              </h3>
            </div>
            <ul className="divide-y divide-gray-200 dark:divide-gray-700">
              {data.top_claims.map((claim) => (
                <li key={claim.claim_excerpt} className="px-6 py-3">
                  <p className="text-sm text-gray-900 dark:text-gray-100">
                    {claim.claim_excerpt}
                  </p>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                    {claim.occurrences} occurrence{claim.occurrences === 1 ? '' : 's'} ·
                    {claim.coach_count} coach{claim.coach_count === 1 ? '' : 'es'} · last seen{' '}
                    {formatTimestamp(claim.last_seen_at)}
                  </p>
                </li>
              ))}
            </ul>
          </Card>

          <Card className="overflow-hidden">
            <div className="border-b border-gray-200 px-6 py-3 dark:border-gray-700">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-on-surface">
                Top offending coaches
              </h3>
            </div>
            <ul className="divide-y divide-gray-200 dark:divide-gray-700">
              {data.top_coaches.map((coach) => (
                <li key={coach.coach_id} className="px-6 py-3">
                  <div className="flex items-start justify-between gap-4">
                    <div className="font-mono text-sm text-gray-900 dark:text-gray-100">
                      {coach.coach_id}
                    </div>
                    <div className="text-sm font-semibold text-gray-900 dark:text-on-surface">
                      {coach.unsupported_total} flagged
                    </div>
                  </div>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                    Categories: {coach.categories.map(humanizeCategory).join(', ') || '—'}
                  </p>
                </li>
              ))}
            </ul>
          </Card>

          <Card className="overflow-hidden">
            <div className="border-b border-gray-200 px-6 py-3 dark:border-gray-700">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-on-surface">
                Top categories
              </h3>
            </div>
            <ul className="divide-y divide-gray-200 dark:divide-gray-700">
              {data.top_categories.map((cat) => (
                <li
                  key={cat.category}
                  className="flex items-center justify-between gap-4 px-6 py-3"
                >
                  <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                    {humanizeCategory(cat.category)}
                  </div>
                  <div className="text-xs text-gray-500 dark:text-gray-400">
                    {cat.flagged_total} flagged · {cat.coach_count} coach
                    {cat.coach_count === 1 ? '' : 'es'}
                  </div>
                </li>
              ))}
            </ul>
          </Card>
        </>
      ) : null}
    </div>
  );
}
