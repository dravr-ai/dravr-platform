// ABOUTME: Phase D Sprint C13 — admin myth-busting tab summarizing unsupported claims
// ABOUTME: Top recurring claim texts, top offending agents, top categories from claim_verdicts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
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
  const queryClient = useQueryClient();
  const [promotionStatus, setPromotionStatus] = useState<{
    topic: string;
    added: boolean;
  } | null>(null);

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery<MythBustingSummary>({
    queryKey: ['admin', 'myth-busting', tenantId] as const,
    queryFn: () => adminApi.getMythBustingSummary(tenantId, 200),
    enabled: Boolean(tenantId),
    refetchInterval: 120_000,
  });

  const promoteMutation = useMutation({
    mutationFn: (topic: string) => adminApi.promoteMythBustingTopic(topic),
    onSuccess: (result) => {
      setPromotionStatus({ topic: result.topic, added: result.added });
      // Harness Config queries (if active) need to refresh so the
      // operator sees the new blocked_topics value without reloading.
      void queryClient.invalidateQueries({ queryKey: ['admin', 'harness-config'] });
    },
  });

  const handlePromote = (claimText: string) => {
    const topic = window.prompt(
      'Block topic — agent replies containing this substring will be rejected:',
      claimText.slice(0, 200),
    );
    if (topic && topic.trim().length > 0) {
      promoteMutation.mutate(topic.trim());
    }
  };

  if (!tenantId) {
    return (
      <Card className="p-6">
        <p className="text-sm text-on-surface-variant">
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
            <h2 className="text-xl font-semibold text-on-surface">
              Myth-busting — recurring unsupported claims
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Pattern view over the latest{' '}
              <span className="font-medium">{data?.verdicts_scanned ?? '—'}</span>{' '}
              claim verdicts. Highlights the recurring unsupported and
              contradicted claims, the agents that emit them, and the
              categories most likely to need myth-busting.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Verdicts scanned
            </div>
            <div className="mt-1 text-2xl font-semibold text-on-surface">
              {data?.verdicts_scanned ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Flagged total
            </div>
            <div className="mt-1 text-2xl font-semibold text-on-surface">
              {data?.flagged_total ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
            <div className="text-xs text-on-surface-variant">
              Distinct claims
            </div>
            <div className="mt-1 text-2xl font-semibold text-on-surface">
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
          <p className="text-sm text-error">
            Failed to load myth-busting summary:{' '}
            {error instanceof Error ? error.message : String(error)}
          </p>
        </Card>
      ) : data && data.flagged_total === 0 ? (
        <Card className="p-12 text-center">
          <p className="text-on-surface-variant">
            No unsupported or contradicted claims in the latest scan window.
          </p>
          <p className="mt-1 text-xs text-outline">
            Either agents are well-behaved, or the claim verifier classified
            everything as supported or rhetorical.
          </p>
        </Card>
      ) : data ? (
        <>
          <Card className="overflow-hidden">
            <div className="border-b border-outline-variant px-6 py-3">
              <h3 className="text-sm font-semibold text-on-surface">
                Top recurring claims
              </h3>
            </div>
            <ul className="divide-y divide-outline-variant">
              {data.top_claims.map((claim) => (
                <li key={claim.claim_excerpt} className="px-6 py-3">
                  <div className="flex items-start justify-between gap-4">
                    <div className="min-w-0 flex-1">
                      <p className="text-sm text-on-surface">
                        {claim.claim_excerpt}
                      </p>
                      <p className="mt-1 text-xs text-on-surface-variant">
                        {claim.occurrences} occurrence{claim.occurrences === 1 ? '' : 's'} ·
                        {claim.coach_count} agent{claim.coach_count === 1 ? '' : 's'} · last seen{' '}
                        {formatTimestamp(claim.last_seen_at)}
                      </p>
                    </div>
                    <Button
                      onClick={() => handlePromote(claim.claim_excerpt)}
                      variant="secondary"
                      disabled={promoteMutation.isPending}
                      data-testid="promote-topic-btn"
                    >
                      {promoteMutation.isPending ? 'Blocking…' : 'Block topic'}
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
            {promotionStatus ? (
              <div
                className="border-t border-outline-variant px-6 py-3 text-xs"
                data-testid="promote-status"
              >
                {promotionStatus.added
                  ? `Topic "${promotionStatus.topic}" added to harness blocked_topics — chat replies containing it will now be rejected.`
                  : `Topic "${promotionStatus.topic}" was already in blocked_topics — no change.`}
              </div>
            ) : promoteMutation.isError ? (
              <div className="border-t border-error px-6 py-3 text-xs text-error">
                Failed to promote topic:{' '}
                {promoteMutation.error instanceof Error
                  ? promoteMutation.error.message
                  : 'unknown error'}
              </div>
            ) : null}
          </Card>

          <Card className="overflow-hidden">
            <div className="border-b border-outline-variant px-6 py-3">
              <h3 className="text-sm font-semibold text-on-surface">
                Top offending agents
              </h3>
            </div>
            <ul className="divide-y divide-outline-variant">
              {data.top_coaches.map((coach) => (
                <li key={coach.coach_id} className="px-6 py-3">
                  <div className="flex items-start justify-between gap-4">
                    <div className="font-mono text-sm text-on-surface">
                      {coach.coach_id}
                    </div>
                    <div className="text-sm font-semibold text-on-surface">
                      {coach.unsupported_total} flagged
                    </div>
                  </div>
                  <p className="mt-1 text-xs text-on-surface-variant">
                    Categories: {coach.categories.map(humanizeCategory).join(', ') || '—'}
                  </p>
                </li>
              ))}
            </ul>
          </Card>

          <Card className="overflow-hidden">
            <div className="border-b border-outline-variant px-6 py-3">
              <h3 className="text-sm font-semibold text-on-surface">
                Top categories
              </h3>
            </div>
            <ul className="divide-y divide-outline-variant">
              {data.top_categories.map((cat) => (
                <li
                  key={cat.category}
                  className="flex items-center justify-between gap-4 px-6 py-3"
                >
                  <div className="text-sm font-medium text-on-surface">
                    {humanizeCategory(cat.category)}
                  </div>
                  <div className="text-xs text-on-surface-variant">
                    {cat.flagged_total} flagged · {cat.coach_count} agent
                    {cat.coach_count === 1 ? '' : 's'}
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
